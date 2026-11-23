//! Exporting an edit: the photo decoded at its own size, the geometry, the
//! tone and the look, the vignette and sharpening worked on every core
//! (rows split among them, as filters.rs and the burst merge split theirs),
//! the marks drawn, and a new JPEG written beside it as `IMG_…_edit.jpg`
//! (`_edit_2`, … for the next). The photo itself is never written to.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use super::develop::{self, Tone};
use super::edits::Edits;
use crate::photos::Picture;

/// Where an edit of `photo` goes: beside it, named for it, never over a
/// file already there and never the photo itself.
pub fn edit_path(photo: &Path) -> PathBuf {
    let dir = photo.parent().unwrap_or(Path::new("."));
    let stem = photo.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "IMG".into());
    for n in 1.. {
        let name = if n == 1 { format!("{stem}_edit.jpg") } else { format!("{stem}_edit_{n}.jpg") };
        let path = dir.join(name);
        if path != photo && !path.exists() && !path.with_extension("jpg.part").exists() {
            return path;
        }
    }
    unreachable!("some name is free")
}

/// The edited picture at the source's own size: cut, turned and
/// straightened, toned, its look laid on, vignetted and sharpened, marked.
/// `progress` hears how far it has come (0 to 1).
pub fn render(src: &Picture, e: &Edits, threads: usize, progress: &dyn Fn(f32)) -> Picture {
    let (tw, th) = develop::turned(src.w, src.h, e.turns);
    let ow = ((e.crop[2] * tw as f32).round() as usize).max(1);
    let oh = ((e.crop[3] * th as f32).round() as usize).max(1);
    let map = develop::mapping((src.w, src.h), e, e.crop, (ow, oh));
    let tone = Tone::of(e);
    let mut out = Picture::new(ow, oh);
    // The geometry and the tone in one pass over each core's rows.
    develop::by_rows(&mut out.rgb, ow * 3, threads, |y0, rows| {
        let n = rows.len() / (ow * 3);
        develop::resample_rows(src, &map, ow, y0..y0 + n, rows);
        tone.develop_rows(ow, oh, y0, rows);
    });
    progress(0.6);
    if tone.sharp > 0 {
        let mut sharp = Picture::new(ow, oh);
        develop::by_rows(&mut sharp.rgb, ow * 3, threads, |y0, rows| develop::sharpen_rows(&out, tone.sharp, y0..y0 + rows.len() / (ow * 3), rows));
        out = sharp;
    }
    progress(0.8);
    super::markup::draw(&mut out, &e.marks);
    progress(1.0);
    out
}

/// The edit of `photo` written as a JPEG: to a part file first, then given
/// its name (taken afresh should that one have appeared meanwhile).
pub fn write(photo: &Path, pic: &Picture, exif: Option<Vec<u8>>) -> Result<PathBuf, String> {
    let path = edit_path(photo);
    let part = path.with_extension("jpg.part");
    let file = std::fs::File::create(&part).map_err(|e| format!("cannot write the edit: {e}"))?;
    let mut encoder = jpeg_encoder::Encoder::new(std::io::BufWriter::new(file), 93);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
    if let Some(exif) = exif {
        encoder.add_app_segment(1, exif).map_err(|e| format!("the edit's details: {e}"))?;
    }
    encoder.encode(&pic.rgb, pic.w as u16, pic.h as u16, jpeg_encoder::ColorType::Rgb).map_err(|e| format!("the edit could not be made: {e}"))?;
    let to = if path.exists() { edit_path(photo) } else { path };
    std::fs::rename(&part, &to).map_err(|e| {
        std::fs::remove_file(&part).ok();
        format!("the edit could not be kept: {e}")
    })?;
    Ok(to)
}

/// An export under way: how far it has come (thousandths) and its answer.
pub struct Job {
    progress: Arc<AtomicU32>,
    answer: Receiver<Result<PathBuf, String>>,
}

impl Job {
    /// 0 to 1.
    pub fn fraction(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// The new photo once written, or why not.
    pub fn done(&self) -> Option<Result<PathBuf, String>> {
        self.answer.try_recv().ok()
    }
}

/// An export held part-way (the shots).
#[cfg(not(target_arch = "aarch64"))]
pub fn held_at(f: f32) -> Job {
    let (_tx, answer) = mpsc::channel();
    Job { progress: Arc::new(AtomicU32::new((f * 1000.0) as u32)), answer }
}

/// Export `photo` with `edits`, behind the panel: decoding (the first
/// quarter of the bar), the work on the cores (to 0.7), writing (the rest).
pub fn start(photo: PathBuf, edits: Edits) -> Job {
    let progress = Arc::new(AtomicU32::new(0));
    let (tx, answer) = mpsc::channel();
    let p = progress.clone();
    std::thread::Builder::new()
        .name("photo-export".into())
        .spawn(move || {
            crate::sched::enter(crate::sched::Class::Batch(10));
            let set = |f: f32| {
                p.store((f.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
                crate::platform::wake();
            };
            let result = run(&photo, &edits, &set);
            set(1.0);
            tx.send(result).ok();
            crate::platform::wake();
        })
        .ok();
    Job { progress, answer }
}

fn run(photo: &Path, edits: &Edits, set: &dyn Fn(f32)) -> Result<PathBuf, String> {
    set(0.02);
    let (w, h) = crate::photos::dimensions(photo).ok_or("the photo cannot be read")?;
    let src = crate::photos::decode(photo, w, h)?;
    set(0.25);
    let pic = render(&src, edits, develop::cores(), &|f| set(0.25 + 0.45 * f));
    drop(src);
    let exif = (edits.look != crate::filters::Look::Original).then(|| crate::filters::exif_with_look(edits.look.name()));
    let path = write(photo, &pic, exif)?;
    crate::photos::changed();
    Ok(path)
}
