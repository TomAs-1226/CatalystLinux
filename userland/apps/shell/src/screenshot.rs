//! Screenshots: the panel as it showed, saved as PNG files in a folder of
//! their own on the data partition (`/data/screenshots`, which Files opens
//! onto), or beside the settings on a computer. Each is saved twice: at the
//! panel's own size, pixel for pixel, and at twice that, for looking at on a
//! phone or putting in a document. Taken from the control centre (Screenshot,
//! or S while it is down); the picture is the screen beneath the sheet.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crate::gfx::{H, W};

/// Where screenshots go.
pub fn folder() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        "/data/screenshots".into()
    } else {
        crate::config_dir().join("screenshots")
    }
}

/// Save `px` (the panel, as it showed) on a thread of its own, so a frame is
/// not kept waiting; the answer (the file, or why not) arrives on the receiver.
pub fn save(px: Vec<u32>) -> mpsc::Receiver<Result<PathBuf, String>> {
    let (tx, rx) = mpsc::channel();
    let name = chrono::Local::now().format("screenshot-%Y%m%d-%H%M%S").to_string();
    std::thread::spawn(move || {
        tx.send(save_now(&px, &folder(), &name)).ok();
    });
    rx
}

/// Save `px` as `<name>.png` and `<name>-2x.png` in `dir`. Returns the first.
pub fn save_now(px: &[u32], dir: &Path, name: &str) -> Result<PathBuf, String> {
    let (w, h) = (W as usize, H as usize);
    if px.len() != w * h {
        return Err("could not take the screenshot".into());
    }
    std::fs::create_dir_all(dir).map_err(|_| "could not make the screenshots folder".to_owned())?;
    // A second in the same name: the next one along.
    let mut stem = name.to_owned();
    let mut n = 2;
    while dir.join(format!("{stem}.png")).exists() {
        stem = format!("{name}-{n}");
        n += 1;
    }
    let path = dir.join(format!("{stem}.png"));
    write_png(px, w, h, 1, &path).map_err(|_| "could not save the screenshot".to_owned())?;
    write_png(px, w, h, 2, &dir.join(format!("{stem}-2x.png"))).map_err(|_| "could not save the screenshot".to_owned())?;
    Ok(path)
}

/// A PNG of the panel, each pixel `scale` pixels square.
fn write_png(px: &[u32], w: usize, h: usize, scale: usize, path: &Path) -> std::io::Result<()> {
    let mut rgb = Vec::with_capacity(w * scale * h * scale * 3);
    for y in 0..h * scale {
        for x in 0..w * scale {
            let p = px[(y / scale) * w + x / scale];
            rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    // Written whole beside the folder's others, then named: a half-written
    // file is never left under a screenshot's name.
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, (w * scale) as u32, (h * scale) as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(std::io::Error::other)?;
        writer.write_image_data(&rgb).map_err(std::io::Error::other)?;
    }
    crate::sys::write_atomic(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screenshot_is_saved_at_the_panels_size_and_twice_it() {
        let dir = std::env::temp_dir().join(format!("catalyst-shots-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let px = vec![0x3366ccu32; (W * H) as usize];
        let first = save_now(&px, &dir, "screenshot-test").unwrap();
        assert_eq!(first.file_name().unwrap(), "screenshot-test.png");
        let read = |p: &Path| {
            let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p).unwrap()));
            let reader = decoder.read_info().unwrap();
            (reader.info().width, reader.info().height)
        };
        assert_eq!(read(&first), (320, 170));
        assert_eq!(read(&dir.join("screenshot-test-2x.png")), (640, 340));
        // Taken again in the same second: a name of its own.
        let second = save_now(&px, &dir, "screenshot-test").unwrap();
        assert_eq!(second.file_name().unwrap(), "screenshot-test-2.png");
        assert!(save_now(&px[..10], &dir, "short").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
