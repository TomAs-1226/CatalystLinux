//! The document scanner: the Camera's "doc" mode (1.0).
//!
//! In the viewfinder the page is found a few times a second on a thread
//! of its own (quad.rs: its edges, then the largest convex quadrilateral)
//! and outlined, its corners gliding to each new finding. Held still for a
//! second it is taken by itself (and again only once the page has moved
//! or gone); Enter takes it at once. The photo is the Camera's own still,
//! at the sensor's full size, written into the work folder; its page is
//! flattened there on a thread (warp.rs: its true shape recovered from the
//! lens and its corners, A4 or Letter guessed from that), and the pages
//! open in a review (review.rs): colour, grey or black and white
//! (enhance.rs), the paper, the order, Add page, Delete. Saved, they are
//! one PDF (pdf.rs) and a PNG a page in `/data/scans`, where Catalyst
//! Companion finds them.
//!
//! The pages wait in `/data/scans/.work` (a list, and each page flattened
//! as a JPEG) until saved, so leaving the Camera loses none.

pub mod enhance;
pub mod pdf;
pub mod quad;
mod review;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
pub mod sim;
#[cfg(test)]
mod tests;
pub mod warp;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bezel::{Color, Rect};
use detent::{role, Animated};

pub use self::enhance::{Enhance, ENHANCES};
use self::quad::Quad;
pub use self::review::Review;
pub use self::warp::{Paper, PAPERS};
use crate::gfx::Gfx;
use crate::photos::Picture;

/// Where scans are kept: on the data partition, beside the photos.
pub fn dir() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        PathBuf::from("/data/scans")
    } else {
        crate::config_dir().join("scans")
    }
}

/// Where the pages wait until saved.
fn work_dir() -> PathBuf {
    dir().join(".work")
}

/// A flattened page's long side, at most: about 200 dpi on A4.
pub const LONGEST: usize = 2200;

/// How long the page must hold still to be taken by itself.
pub const HOLD: f32 = 1.0;

/// How often the page is looked for.
const FIND_EVERY: Duration = Duration::from_millis(200);

// ------------------------------------------------------------- the pages ---

/// A page of the document.
pub struct Page {
    pub id: String,
    pub enhance: Enhance,
    pub paper: Paper,
    /// Its shape as measured (height over width) and its flattened size.
    pub ratio: f32,
    pub size: (usize, usize),
    /// The page flattened, small (for the review), once it is.
    pub preview: Option<Picture>,
    pub trouble: Option<String>,
    /// The preview as shown (at its paper, enhanced) and what for.
    shown: Option<((Enhance, Paper), Picture)>,
}

impl Page {
    /// The paper it comes to.
    pub fn paper_now(&self) -> Paper {
        self.paper.resolve(self.ratio.max(1.0 / self.ratio))
    }

    /// Its height over its width on that paper.
    pub fn shape(&self) -> f32 {
        match self.paper_now().ratio() {
            Some(r) if self.ratio >= 1.0 => r,
            Some(r) => 1.0 / r,
            None => self.ratio,
        }
    }

    /// The page as the review shows it: at its paper's shape, enhanced.
    pub fn shown(&mut self) -> Option<&Picture> {
        let key = (self.enhance, self.paper_now());
        if self.shown.as_ref().map(|(k, _)| *k) != Some(key) {
            let small = self.preview.as_ref()?;
            let h = ((small.w as f32 * self.shape()).round() as usize).max(1);
            let fitted = resize(small, small.w, h);
            self.shown = Some((key, enhance::enhance(&fitted, self.enhance)));
        }
        self.shown.as_ref().map(|(_, p)| p)
    }
}

/// A picture resized to `w` × `h` (its shape may change), sampled smoothly.
pub fn resize(src: &Picture, w: usize, h: usize) -> Picture {
    let whole = [(0.0, 0.0), (src.w as f32, 0.0), (src.w as f32, src.h as f32), (0.0, src.h as f32)];
    warp::flatten(src, &whole, w, h, crate::editor::cores())
}

/// What a page's flattening gives: the small picture, its shape and size.
type Flattened = Result<(Picture, f32, (usize, usize)), String>;

/// The document being scanned.
pub struct Doc {
    pub pages: Vec<Page>,
    jobs: Vec<(String, Receiver<Flattened>)>,
    saving: Option<Saving>,
    /// The last document saved: its PDF and how many pages.
    pub saved: Option<(PathBuf, usize)>,
    pub trouble: Option<String>,
}

/// A save under way: how far (thousandths), and its answer.
struct Saving {
    progress: Arc<AtomicU32>,
    answer: Receiver<Result<PathBuf, String>>,
    pages: usize,
}

impl Doc {
    /// The document waiting in the work folder (none: an empty one).
    pub fn open() -> Doc {
        let mut doc = Doc { pages: Vec::new(), jobs: Vec::new(), saving: None, saved: None, trouble: None };
        let text = std::fs::read_to_string(work_dir().join("doc.txt")).unwrap_or_default();
        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let ["page", id, how, paper, ratio, w, h] = parts[..] {
                if !work_dir().join(format!("{id}.jpg")).is_file() {
                    continue;
                }
                let size = (w.parse().unwrap_or(0), h.parse().unwrap_or(0));
                doc.pages.push(Page { id: id.to_owned(), enhance: Enhance::from_id(how), paper: Paper::from_name(paper), ratio: ratio.parse().unwrap_or(1.414), size, preview: None, trouble: None, shown: None });
            }
        }
        // Their small pictures, read back on a thread.
        for page in &doc.pages {
            let (tx, rx) = mpsc::channel();
            let (path, ratio, size) = (work_dir().join(format!("{}.jpg", page.id)), page.ratio, page.size);
            std::thread::Builder::new()
                .name("doc-page".into())
                .spawn(move || {
                    let small = crate::photos::decode(&path, 300, 300).map(|p| (shrink(&p, 360), ratio, size));
                    tx.send(small).ok();
                    crate::platform::wake();
                })
                .ok();
            doc.jobs.push((page.id.clone(), rx));
        }
        doc
    }

    /// The pages' list written (or the work folder emptied, with none).
    fn keep(&self) {
        let work = work_dir();
        if self.pages.is_empty() {
            std::fs::remove_file(work.join("doc.txt")).ok();
            return;
        }
        std::fs::create_dir_all(&work).ok();
        let text: String = self.pages.iter().map(|p| format!("page {} {} {} {:.4} {} {}\n", p.id, p.enhance.id(), p.paper.name(), p.ratio, p.size.0, p.size.1)).collect();
        crate::sys::write_atomic(&work.join("doc.txt"), text.as_bytes()).ok();
    }

    /// A photo taken of a page (its corners 0 to 1 of the photo): a page
    /// added, flattened on a thread. It takes the last page's enhancement.
    pub fn add(&mut self, shot: PathBuf, corners: Quad) {
        let id = shot.file_stem().map(|s| s.to_string_lossy().trim_end_matches("_shot").to_owned()).unwrap_or_else(|| "page".into());
        let enhance = self.pages.last().map_or(Enhance::Mono, |p| p.enhance);
        self.pages.push(Page { id: id.clone(), enhance, paper: Paper::Auto, ratio: 1.414, size: (0, 0), preview: None, trouble: None, shown: None });
        let (tx, rx) = mpsc::channel();
        let flat = work_dir().join(format!("{id}.jpg"));
        std::thread::Builder::new()
            .name("doc-page".into())
            .spawn(move || {
                crate::sched::enter(crate::sched::Class::Batch(10));
                let result = flatten_shot(&shot, &corners, &flat);
                std::fs::remove_file(&shot).ok();
                tx.send(result).ok();
                crate::platform::wake();
            })
            .ok();
        self.jobs.push((id, rx));
        self.keep();
    }

    /// Whether a page is being flattened (or read back).
    pub fn working(&self) -> bool {
        !self.jobs.is_empty() || self.saving.is_some()
    }

    /// What finished: pages flattened, the save. True when anything did.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.jobs.len() {
            let Ok(result) = self.jobs[i].1.try_recv() else {
                i += 1;
                continue;
            };
            let (id, _) = self.jobs.remove(i);
            if let Some(page) = self.pages.iter_mut().find(|p| p.id == id) {
                match result {
                    Ok((small, ratio, size)) => {
                        page.preview = Some(small);
                        page.ratio = ratio;
                        page.size = size;
                        page.shown = None;
                    }
                    Err(why) => page.trouble = Some(why),
                }
            }
            self.pages.retain(|p| p.trouble.is_none() || p.preview.is_some());
            self.keep();
            changed = true;
        }
        if let Some(s) = &self.saving {
            if let Ok(result) = s.answer.try_recv() {
                let n = s.pages;
                self.saving = None;
                match result {
                    Ok(pdf) => {
                        self.saved = Some((pdf, n));
                        self.pages.clear();
                        std::fs::remove_dir_all(work_dir()).ok();
                    }
                    Err(why) => self.trouble = Some(why),
                }
                changed = true;
            }
        }
        changed
    }

    /// How far the save has come (0 to 1), while it runs.
    pub fn saving(&self) -> Option<f32> {
        self.saving.as_ref().map(|s| s.progress.load(Ordering::Relaxed) as f32 / 1000.0)
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.pages.len() {
            let page = self.pages.remove(i);
            std::fs::remove_file(work_dir().join(format!("{}.jpg", page.id))).ok();
            self.keep();
        }
    }

    /// A page moved earlier (−1) or later (1); where it went.
    pub fn shift(&mut self, i: usize, by: isize) -> usize {
        let to = (i as isize + by).clamp(0, self.pages.len().saturating_sub(1) as isize) as usize;
        if to != i && i < self.pages.len() {
            self.pages.swap(i, to);
            self.keep();
        }
        to
    }

    pub fn set(&mut self, i: usize, enhance: Option<Enhance>, paper: Option<Paper>) {
        if let Some(page) = self.pages.get_mut(i) {
            if let Some(e) = enhance {
                page.enhance = e;
            }
            if let Some(p) = paper {
                page.paper = p;
            }
            self.keep();
        }
    }

    /// The document saved, on a thread: a PDF and a PNG a page in the
    /// scans folder, named for now. False when there is nothing to save
    /// yet (no pages, or one still being flattened).
    pub fn save(&mut self) -> bool {
        if self.pages.is_empty() || !self.jobs.is_empty() || self.saving.is_some() {
            return false;
        }
        let pages: Vec<(PathBuf, Enhance, f32, Paper, f32)> = self.pages.iter().map(|p| (work_dir().join(format!("{}.jpg", p.id)), p.enhance, p.shape(), p.paper_now(), p.ratio)).collect();
        let progress = Arc::new(AtomicU32::new(0));
        let (tx, answer) = mpsc::channel();
        let p = progress.clone();
        let n = pages.len();
        std::thread::Builder::new()
            .name("doc-save".into())
            .spawn(move || {
                crate::sched::enter(crate::sched::Class::Batch(10));
                let result = save_pages(&dir(), &pages, chrono::Local::now().naive_local(), &|f| {
                    p.store((f.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
                    crate::platform::wake();
                });
                tx.send(result).ok();
                crate::platform::wake();
            })
            .ok();
        self.saving = Some(Saving { progress, answer, pages: n });
        self.trouble = None;
        true
    }
}

/// A picture shrunk so its long side is at most `long` (each pixel the mean
/// of those it stands for).
pub fn shrink(pic: &Picture, long: usize) -> Picture {
    let k = long as f32 / pic.w.max(pic.h) as f32;
    if k >= 1.0 {
        return pic.clone();
    }
    pic.cover(((pic.w as f32 * k).round() as usize).max(1), ((pic.h as f32 * k).round() as usize).max(1))
}

/// A photo of a page flattened: decoded at its own size, its page found
/// (corners 0 to 1 of it), drawn flat at its true shape and kept as a JPEG
/// in the work folder. Gives the page small, its shape and its size.
pub fn flatten_shot(shot: &Path, corners: &Quad, out: &Path) -> Flattened {
    let (fw, fh) = crate::photos::dimensions(shot).ok_or("the photo cannot be read")?;
    let src = crate::photos::decode(shot, fw, fh)?;
    // A hair inside the page's edge, so no sliver of what it lies on comes along.
    let (cx, cy) = (corners.iter().map(|p| p.0).sum::<f32>() / 4.0, corners.iter().map(|p| p.1).sum::<f32>() / 4.0);
    let q = corners.map(|(x, y)| ((cx + (x - cx) * 0.993) * fw as f32, (cy + (y - cy) * 0.993) * fh as f32));
    let focal = warp::FOCAL_FULL * fw as f32 / crate::camera::FULL.0 as f32;
    let ratio = warp::page_ratio(&q, (fw as f32, fh as f32), focal);
    let (w, h) = warp::flat_size(&q, ratio, LONGEST);
    let flat = warp::flatten(&src, &q, w, h, crate::editor::cores());
    drop(src);
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot keep the page: {e}"))?;
    }
    write_jpeg(out, &flat, 92, false)?;
    Ok((shrink(&flat, 360), ratio, (w, h)))
}

/// A picture as a JPEG file (written whole, then named), in grey when asked.
fn write_jpeg(path: &Path, pic: &Picture, quality: u8, grey: bool) -> Result<(), String> {
    let bytes = jpeg_bytes(pic, quality, grey)?;
    let part = path.with_extension("part");
    std::fs::write(&part, bytes).map_err(|e| format!("cannot write the page: {e}"))?;
    std::fs::rename(&part, path).map_err(|e| format!("cannot keep the page: {e}"))
}

/// A picture as JPEG bytes: in colour, or its first channel as grey.
pub fn jpeg_bytes(pic: &Picture, quality: u8, grey: bool) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    let made = if grey {
        let luma: Vec<u8> = pic.rgb.chunks_exact(3).map(|p| p[0]).collect();
        encoder.encode(&luma, pic.w as u16, pic.h as u16, jpeg_encoder::ColorType::Luma)
    } else {
        encoder.encode(&pic.rgb, pic.w as u16, pic.h as u16, jpeg_encoder::ColorType::Rgb)
    };
    made.map_err(|e| format!("the page could not be made: {e}"))?;
    Ok(out)
}

/// A PNG of a page: colour, grey, or a bit a pixel.
pub fn png_bytes(pic: &Picture, how: Enhance) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, pic.w as u32, pic.h as u32);
        let data = match how {
            Enhance::Colour => {
                enc.set_color(png::ColorType::Rgb);
                enc.set_depth(png::BitDepth::Eight);
                pic.rgb.clone()
            }
            Enhance::Grey => {
                enc.set_color(png::ColorType::Grayscale);
                enc.set_depth(png::BitDepth::Eight);
                pic.rgb.chunks_exact(3).map(|p| p[0]).collect()
            }
            Enhance::Mono => {
                enc.set_color(png::ColorType::Grayscale);
                enc.set_depth(png::BitDepth::One);
                let paper: Vec<bool> = pic.rgb.chunks_exact(3).map(|p| p[0] > 127).collect();
                pdf::pack_bits(&paper, pic.w, pic.h)
            }
        };
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&data).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// The scan's name for `now`, never one already there: `SCAN_20261122_141500`.
pub fn scan_stem(dir: &Path, now: chrono::NaiveDateTime) -> String {
    let base = format!("SCAN_{}", now.format("%Y%m%d_%H%M%S"));
    let mut stem = base.clone();
    let mut n = 2;
    while dir.join(format!("{stem}.pdf")).exists() || dir.join(format!("{stem}_1.png")).exists() {
        stem = format!("{base}_{n}");
        n += 1;
    }
    stem
}

/// The pages saved into `dir`: each flattened page read, set to its paper's
/// shape, enhanced, written as a PNG and put in the PDF, which is written
/// last. Gives the PDF's path.
pub fn save_pages(dir: &Path, pages: &[(PathBuf, Enhance, f32, Paper, f32)], now: chrono::NaiveDateTime, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make the scans folder: {e}"))?;
    let stem = scan_stem(dir, now);
    let mut made = Vec::new();
    let mut written = Vec::new();
    let n = pages.len().max(1) as f32;
    for (i, (flat, how, shape, paper, _)) in pages.iter().enumerate() {
        progress(i as f32 / n);
        let pic = crate::photos::decode(flat, LONGEST, LONGEST)?;
        let h = ((pic.w as f32 * shape).round() as usize).max(1);
        let page = if h == pic.h { pic } else { resize(&pic, pic.w, h) };
        let page = enhance::enhance(&page, *how);
        let png = dir.join(format!("{stem}_{}.png", i + 1));
        std::fs::write(&png, png_bytes(&page, *how)?).map_err(|e| format!("cannot write a page: {e}"))?;
        written.push(png);
        // The PDF's page: its paper's size, else A4's width at its shape.
        let size = paper.points().map_or((595.28, 595.28 * shape), |(w, h)| if *shape >= 1.0 { (w, h) } else { (h, w) });
        let image = match how {
            Enhance::Mono => pdf::Image::Bits { rows: pdf::pack_bits(&page.rgb.chunks_exact(3).map(|p| p[0] > 127).collect::<Vec<_>>(), page.w, page.h), w: page.w, h: page.h },
            Enhance::Grey => pdf::Image::Jpeg { data: jpeg_bytes(&page, 85, true)?, w: page.w, h: page.h, grey: true },
            Enhance::Colour => pdf::Image::Jpeg { data: jpeg_bytes(&page, 85, false)?, w: page.w, h: page.h, grey: false },
        };
        made.push(pdf::Page { size, image });
    }
    progress(0.95);
    let pdf_path = dir.join(format!("{stem}.pdf"));
    let part = pdf_path.with_extension("pdf.part");
    let written_pdf = std::fs::write(&part, pdf::write(&made, &stem)).and_then(|()| std::fs::rename(&part, &pdf_path));
    if let Err(e) = written_pdf {
        for p in written {
            std::fs::remove_file(p).ok();
        }
        return Err(format!("cannot write the pdf: {e}"));
    }
    progress(1.0);
    Ok(pdf_path)
}

// ------------------------------------------------------------ the finder ---

#[derive(Default)]
struct Slot {
    frame: Option<(Vec<u8>, usize, usize)>,
    found: Option<Option<Quad>>,
    stop: bool,
}

/// The page looked for on a thread of its own, a frame at a time, at most
/// five times a second.
pub struct Finder {
    shared: Arc<(Mutex<Slot>, Condvar)>,
    given: Option<Instant>,
}

impl Finder {
    pub fn new() -> Finder {
        let shared: Arc<(Mutex<Slot>, Condvar)> = Arc::default();
        let s = shared.clone();
        std::thread::Builder::new()
            .name("doc-find".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                crate::sched::enter(crate::sched::Class::Batch(10));
                let (lock, ready) = &*s;
                loop {
                    let (y, w, h) = {
                        let Ok(mut slot) = lock.lock() else { return };
                        while slot.frame.is_none() && !slot.stop {
                            slot = match ready.wait(slot) {
                                Ok(s) => s,
                                Err(_) => return,
                            };
                        }
                        if slot.stop {
                            return;
                        }
                        let Some(frame) = slot.frame.take() else { continue };
                        frame
                    };
                    let found = quad::find(&y, w, h);
                    if let Ok(mut slot) = lock.lock() {
                        slot.found = Some(found);
                    }
                    crate::platform::wake();
                }
            })
            .ok();
        Finder { shared, given: None }
    }

    /// Ready for a frame: the last one looked at, and a fifth of a second gone.
    pub fn wants(&self) -> bool {
        self.given.is_none_or(|t| t.elapsed() >= FIND_EVERY) && self.shared.0.lock().map(|s| s.frame.is_none()).unwrap_or(false)
    }

    /// A frame's brightness to look in.
    pub fn give(&mut self, y: &[u8], w: usize, h: usize) {
        if let Ok(mut slot) = self.shared.0.lock() {
            slot.frame = Some((y.to_vec(), w, h));
            self.given = Some(Instant::now());
        }
        self.shared.1.notify_one();
    }

    /// What was found since last asked.
    pub fn take(&mut self) -> Option<Option<Quad>> {
        self.shared.0.lock().ok().and_then(|mut s| s.found.take())
    }
}

impl Drop for Finder {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.shared.0.lock() {
            slot.stop = true;
        }
        self.shared.1.notify_one();
    }
}

// ----------------------------------------------------- in the viewfinder ---

/// The Camera's doc mode: the page found and outlined, held still and
/// taken, and the document its pages go into.
pub struct Finding {
    finder: Finder,
    pub doc: Rc<RefCell<Doc>>,
    /// The preview's size, the page as last found in it, and how long it
    /// has been missing.
    size: (f32, f32),
    quad: Option<Quad>,
    lost_for: f32,
    /// Its outline as drawn: the corners gliding to each finding, its fade.
    corners: [Animated; 8],
    seen: Animated,
    /// How long it has held still, and whether it may be taken by itself
    /// (once the page moved or went since the last).
    still_for: f32,
    armed: bool,
    /// A photo asked for: its file and the page in it (0 to 1).
    pub shot: Option<(PathBuf, Quad)>,
    open: bool,
}

impl Finding {
    pub fn new() -> Finding {
        let spring = |v: f32| Animated::new(v, role::hold()).with_rest(0.05);
        Finding {
            finder: Finder::new(),
            doc: Rc::new(RefCell::new(Doc::open())),
            size: (320.0, 240.0),
            quad: None,
            lost_for: 0.0,
            corners: std::array::from_fn(|_| spring(0.0)),
            seen: Animated::new(0.0, role::effect()).with_rest(0.004),
            still_for: 0.0,
            armed: true,
            shot: None,
            open: false,
        }
    }

    /// A page set where it is (the shots): found, held still this long.
    pub fn place(&mut self, q: Quad, still_for: f32) {
        self.quad = Some(q);
        for (i, (x, y)) in q.iter().enumerate() {
            self.corners[i * 2].jump(*x);
            self.corners[i * 2 + 1].jump(*y);
        }
        self.seen.jump(1.0);
        self.still_for = still_for;
    }

    pub fn wants(&self) -> bool {
        self.shot.is_none() && self.finder.wants()
    }

    /// The preview's brightness, for the finder.
    pub fn give(&mut self, y: &[u8], w: usize, h: usize) {
        self.size = (w as f32, h as f32);
        self.finder.give(y, w, h);
    }

    /// Time passing and what the finder found: whether to draw, and whether
    /// the page has held still long enough to be taken.
    pub fn ambient(&mut self, dt: f32) -> (bool, bool) {
        let mut changed = false;
        let diag = (self.size.0.powi(2) + self.size.1.powi(2)).sqrt();
        if let Some(found) = self.finder.take() {
            match found {
                Some(q) => {
                    let moved = self.quad.map_or(diag, |old| quad::moved(&old, &q));
                    if moved > diag * 0.02 {
                        self.still_for = 0.0;
                    }
                    if moved > diag * 0.08 {
                        self.armed = true;
                    }
                    if self.quad.is_none() {
                        for (i, (x, y)) in q.iter().enumerate() {
                            self.corners[i * 2].jump(*x);
                            self.corners[i * 2 + 1].jump(*y);
                        }
                    }
                    for (i, (x, y)) in q.iter().enumerate() {
                        self.corners[i * 2].set(*x);
                        self.corners[i * 2 + 1].set(*y);
                    }
                    self.quad = Some(q);
                    self.lost_for = 0.0;
                    self.seen.set(1.0);
                }
                None => self.lost_for += FIND_EVERY.as_secs_f32(),
            }
            changed = true;
        }
        // Gone for a moment: the outline fades, and the next page may be taken.
        if self.quad.is_some() && self.lost_for > 0.6 {
            self.quad = None;
            self.still_for = 0.0;
            self.armed = true;
            self.seen.set(0.0);
            changed = true;
        }
        let mut take = false;
        if self.quad.is_some() && self.armed && self.shot.is_none() {
            self.still_for += dt;
            changed = true;
            if self.still_for >= HOLD {
                take = true;
                self.armed = false;
            }
        }
        (changed, take)
    }

    /// The outline's springs.
    pub fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.seen.tick(dt);
        for c in &mut self.corners {
            moving |= c.tick(dt);
        }
        moving
    }

    /// How far through holding still the page is (0 to 1), while it counts.
    pub fn hold(&self) -> Option<f32> {
        (self.quad.is_some() && self.armed && self.shot.is_none()).then(|| (self.still_for / HOLD).clamp(0.0, 1.0))
    }

    /// The page to take now, as 0 to 1 of the picture (the one found, or
    /// none: then the whole picture is the page).
    pub fn corners(&self) -> Quad {
        let (w, h) = self.size;
        self.quad.map_or([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], |q| q.map(|(x, y)| (x / w, y / h)))
    }

    /// Where the next page's photo goes: the work folder, named for now.
    pub fn shot_path() -> PathBuf {
        let work = work_dir();
        std::fs::create_dir_all(&work).ok();
        let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
        work.join(format!("page_{stamp}_shot.jpg"))
    }

    /// A photo saved: if it was this mode's, its page goes into the document
    /// and the review opens. True when it was.
    pub fn took(&mut self, path: &Path) -> bool {
        match self.shot.take() {
            Some((p, q)) if p == path => {
                self.doc.borrow_mut().add(p, q);
                self.open = true;
                true
            }
            other => {
                self.shot = other;
                false
            }
        }
    }

    /// The photo was not taken after all.
    pub fn failed(&mut self) {
        self.shot = None;
        self.armed = true;
        self.still_for = 0.0;
    }

    /// Whether the review is to open now.
    pub fn take_open(&mut self) -> bool {
        std::mem::take(&mut self.open)
    }

    /// The outline over the viewfinder `vf`: the page's edge in the signal
    /// colour over a faint fill of it, and while it holds still the fill
    /// deepening toward the moment it is taken.
    pub fn draw(&self, g: &mut Gfx, vf: Rect, signal: Color) {
        let seen = self.seen.value().clamp(0.0, 1.0);
        if seen < 0.01 {
            return;
        }
        let (sx, sy) = (vf.w / self.size.0, vf.h / self.size.1);
        let pts: Vec<(f32, f32)> = (0..4).map(|i| (vf.x + self.corners[i * 2].value() * sx + g.ox, vf.y + self.corners[i * 2 + 1].value() * sy)).collect();
        let hold = self.hold().unwrap_or(if self.shot.is_some() { 1.0 } else { 0.0 });
        g.canvas.polygon(&pts, signal, seen * (0.12 + 0.22 * hold));
        let mut ring = pts.clone();
        ring.push(pts[0]);
        g.canvas.polyline(&ring, 2.0, signal, seen);
        for (x, y) in &pts {
            g.canvas.circle(*x, *y, 3.5, Color(255, 255, 255), seen);
            g.canvas.circle(*x, *y, 2.0, signal, seen);
        }
    }
}
