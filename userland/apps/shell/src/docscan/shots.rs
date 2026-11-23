//! The scanner's pictures (`catalyst-shell --scan-shots DIR`): the Camera
//! finding a page and holding it, a page just taken, enhanced, the pages
//! of a document, the Camera with pages waiting, and saved; dark and
//! light at each text size, a contact sheet of each, and what was saved
//! (the PDF and its PNGs) beside them.

use std::path::Path;

use super::{quad, Doc, Enhance, Finding, Paper, Review};
use crate::gfx::TextSize;
use crate::shell::{Screen, Shell};

/// A photo of the simulated page taken at `t`, its page found as the
/// Camera finds it (in the preview), added to the document.
fn take(doc: &mut Doc, t: f32) {
    let (fw, fh) = crate::camera::FULL;
    let mut full = crate::camera::scene(fw, fh, t, false);
    super::sim::paint(&mut full, [0.0, 0.0, 1.0, 1.0], t);
    let preview = full.cover(320, 240);
    let luma = crate::camera::brightness_of(&preview);
    let q = quad::find(&luma, 320, 240).expect("the page is found");
    let shot = Finding::shot_path();
    crate::camera::save_jpeg(&shot, &full).expect("the photo");
    doc.add(shot, q.map(|(x, y)| (x / 320.0, y / 240.0)));
    std::thread::sleep(std::time::Duration::from_millis(5));
}

fn settled(doc: &mut Doc) {
    let began = std::time::Instant::now();
    while doc.working() && began.elapsed().as_secs() < 60 {
        doc.poll();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    doc.poll();
}

/// The document as left in the work folder, read back whole.
fn opened() -> std::rc::Rc<std::cell::RefCell<Doc>> {
    let mut doc = Doc::open();
    settled(&mut doc);
    std::rc::Rc::new(std::cell::RefCell::new(doc))
}

pub fn shots(dir: &Path) {
    std::fs::create_dir_all(dir).expect("the shots folder");
    let state = std::env::temp_dir().join(format!("catalyst-scan-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&state).ok();
    std::fs::create_dir_all(&state).expect("a state folder");
    std::env::set_var("CATALYST_CONF", state.join("robot"));
    std::fs::create_dir_all(crate::photos::dir()).ok();
    std::fs::write(crate::photos::dir().join(".seeded"), b"").ok();
    // Three pages, taken a moment apart, as colour, black and white, grey.
    let mut doc = Doc::open();
    for t in [2.0, 9.0, 17.0] {
        take(&mut doc, t);
    }
    settled(&mut doc);
    doc.set(0, Some(Enhance::Colour), None);
    doc.set(1, Some(Enhance::Mono), None);
    doc.set(2, Some(Enhance::Grey), Some(Paper::Letter));
    let one_page = |focus: usize, how: Enhance| {
        move || -> Box<dyn Screen> {
            let doc = opened();
            {
                let mut d = doc.borrow_mut();
                d.pages.truncate(1);
                d.pages[0].enhance = how;
            }
            Box::new(Review::new(doc).at(0, focus))
        }
    };
    type Make = Box<dyn Fn() -> Box<dyn Screen>>;
    let pages: Vec<(&str, Make)> = vec![
        ("scan-1-finding", Box::new(|| Box::new(crate::camera::doc_camera(0.0, 4.0)) as Box<dyn Screen>)),
        ("scan-2-holding", Box::new(|| Box::new(crate::camera::doc_camera(0.62, 4.0)) as Box<dyn Screen>)),
        ("scan-3-captured", Box::new(one_page(0, Enhance::Colour))),
        ("scan-4-enhanced", Box::new(one_page(1, Enhance::Mono))),
        ("scan-5-pages", Box::new(|| Box::new(Review::new(opened()).at(1, 0)) as Box<dyn Screen>)),
        ("scan-6-pages-paper", Box::new(|| Box::new(Review::new(opened()).at(2, 2)) as Box<dyn Screen>)),
        (
            "scan-7-saved",
            Box::new(|| {
                let doc = Doc { pages: Vec::new(), jobs: Vec::new(), saving: None, saved: Some((super::dir().join("SCAN_20261122_141500.pdf"), 3)), trouble: None };
                Box::new(Review::new(std::rc::Rc::new(std::cell::RefCell::new(doc)))) as Box<dyn Screen>
            }),
        ),
    ];
    let mut sheets: Vec<Vec<bezel::Canvas>> = (0..pages.len()).map(|_| Vec::new()).collect();
    for light in [false, true] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            for (n, (name, make)) in pages.iter().enumerate() {
                let mut shell: Shell = crate::fullheight::shell("catalyst", size);
                shell.settings.light = light;
                shell.apply_settings();
                crate::fullheight::settle(&mut shell, 30);
                shell.show(make());
                crate::fullheight::settle(&mut shell, 90);
                shell.draw();
                let file = format!("{name}-{}-{}.png", if light { "light" } else { "dark" }, size.name());
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(file), 2);
                sheets[n].push(shell.gfx.canvas.clone());
            }
        }
    }
    for ((name, _), shots) in pages.iter().zip(&sheets) {
        crate::platform::sim::save_png(&crate::editor::shots::sheet(shots), &dir.join(format!("sheet-{name}.png")), 2);
    }
    // The document saved for real: its PDF and PNGs, beside the shots.
    let doc = opened();
    assert!(doc.borrow_mut().save(), "the document saves");
    let began = std::time::Instant::now();
    while doc.borrow().saving().is_some() && began.elapsed().as_secs() < 120 {
        doc.borrow_mut().poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    doc.borrow_mut().poll();
    if let Some((pdf, _)) = &doc.borrow().saved {
        for entry in std::fs::read_dir(super::dir()).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(pdf.file_stem().unwrap().to_string_lossy().as_ref()) {
                std::fs::copy(entry.path(), dir.join(format!("scan-saved-{name}"))).ok();
            }
        }
    }
    std::fs::remove_dir_all(&state).ok();
    println!("the scanner's pages in {}", dir.display());
}
