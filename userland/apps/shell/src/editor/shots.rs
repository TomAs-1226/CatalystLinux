//! The editor's pictures (`catalyst-shell --editor-shots DIR`): each panel,
//! the crop and the straightening, the markup being placed and typed, an
//! export under way and the photo as it was, dark and light at each text
//! size; and a contact sheet of each (dark above light; compact, standard,
//! large), with the full-size export of the marked-up edit beside them.

use std::path::Path;

use super::edits::{Adjust, Aspect, Ink, Mark, Shape};
use super::{export, CropTool, Editor, Panel, Placing};
use crate::filters::Look;
use crate::gfx::TextSize;
use crate::shell::Shell;

fn marks() -> Vec<Mark> {
    vec![
        Mark { shape: Shape::Arrow, ink: Ink::Signal, a: (0.16, 0.22), b: (0.4, 0.5), text: String::new() },
        Mark { shape: Shape::Circle, ink: Ink::Signal, a: (0.42, 0.46), b: (0.66, 0.78), text: String::new() },
        Mark { shape: Shape::Box, ink: Ink::Ice, a: (0.68, 0.18), b: (0.94, 0.44), text: String::new() },
        Mark { shape: Shape::Text, ink: Ink::Signal, a: (0.24, 0.13), b: (0.24, 0.13), text: "Bumper bolt".into() },
    ]
}

pub fn shots(dir: &Path) {
    std::fs::create_dir_all(dir).expect("the shots folder");
    let state = std::env::temp_dir().join(format!("catalyst-editor-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&state).ok();
    std::fs::create_dir_all(state.join("photos")).expect("a state folder");
    std::env::set_var("CATALYST_CONF", state.join("robot"));
    let photo = crate::photos::dir().join("IMG_20261121_101500.jpg");
    std::fs::create_dir_all(photo.parent().unwrap()).ok();
    crate::camera::save_jpeg(&photo, &crate::camera::scene(1640, 1232, 5.0, false)).expect("the photo");
    std::fs::write(crate::photos::dir().join(".seeded"), b"").ok();

    type Setup = Box<dyn Fn(&mut Editor)>;
    let pages: Vec<(&str, Setup)> = vec![
        (
            "editor-1-adjust",
            Box::new(|e: &mut Editor| {
                for (a, v) in [(Adjust::Exposure, 20), (Adjust::Contrast, 25), (Adjust::Highlights, -35), (Adjust::Shadows, 30), (Adjust::Warmth, 15)] {
                    e.edits.set(a, v);
                }
                e.lists[0].focus = 2;
            }),
        ),
        (
            "editor-2-adjust-end",
            Box::new(|e: &mut Editor| {
                for (a, v) in [(Adjust::Saturation, -20), (Adjust::Vibrance, 40), (Adjust::Sharpness, 35), (Adjust::Vignette, -45)] {
                    e.edits.set(a, v);
                }
                e.lists[0].focus = 9;
            }),
        ),
        (
            "editor-3-looks",
            Box::new(|e: &mut Editor| {
                e.edits.look = Look::VividWarm;
                e.set_panel(Panel::Looks);
            }),
        ),
        (
            "editor-4-crop",
            Box::new(|e: &mut Editor| {
                e.set_panel(Panel::Geometry);
                e.edits.aspect = Aspect::SixteenNine;
                e.edits.crop = super::aspect_box(Aspect::SixteenNine, e.frame(), (0.55, 0.55), 0.8);
                e.crop_tool = Some(CropTool::Move);
            }),
        ),
        (
            "editor-5-straighten",
            Box::new(|e: &mut Editor| {
                e.set_panel(Panel::Geometry);
                e.lists[2].focus = 2;
                e.edits.straighten = 4.5;
            }),
        ),
        (
            "editor-6-turned",
            Box::new(|e: &mut Editor| {
                e.set_panel(Panel::Geometry);
                e.lists[2].focus = 3;
                e.edits.turns = 1;
                e.edits.flip_h = true;
                e.edits.aspect = Aspect::Square;
                e.edits.crop = super::aspect_box(Aspect::Square, e.frame(), (0.5, 0.5), 1.0);
            }),
        ),
        (
            "editor-7-markup",
            Box::new(|e: &mut Editor| {
                e.set_panel(Panel::Markup);
                e.edits.marks = marks();
            }),
        ),
        (
            "editor-8-markup-placing",
            Box::new(|e: &mut Editor| {
                e.set_panel(Panel::Markup);
                e.edits.marks = marks()[..1].to_vec();
                e.tool = Shape::Box;
                e.ink = Ink::Ice;
                e.lists[3].focus = 1;
                e.placing = Some(Placing::End((0.62, 0.2)));
                e.cursor = (0.9, 0.5);
            }),
        ),
        (
            "editor-9-markup-typing",
            Box::new(|e: &mut Editor| {
                e.set_panel(Panel::Markup);
                e.edits.marks = marks()[..3].to_vec();
                e.tool = Shape::Text;
                e.placing = Some(Placing::Typing((0.3, 0.86), "Left intake".into()));
            }),
        ),
        (
            "editor-10-exporting",
            Box::new(|e: &mut Editor| {
                e.edits.set(Adjust::Contrast, 20);
                e.edits.look = Look::Contemporary;
                e.edits.marks = marks();
                e.export = Some(export::held_at(0.46));
                e.bar.jump(0.46);
            }),
        ),
        (
            "editor-11-before",
            Box::new(|e: &mut Editor| {
                e.edits.look = Look::Noir;
                e.before = true;
            }),
        ),
    ];
    let mut sheets: Vec<Vec<bezel::Canvas>> = (0..pages.len()).map(|_| Vec::new()).collect();
    for light in [false, true] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            for (n, (name, setup)) in pages.iter().enumerate() {
                std::fs::remove_dir_all(crate::photos::dir().join(".edits")).ok();
                let mut shell: Shell = crate::fullheight::shell("catalyst", size);
                shell.settings.light = light;
                shell.apply_settings();
                crate::fullheight::settle(&mut shell, 30);
                let mut editor = Editor::open_now(photo.clone());
                setup(&mut editor);
                editor.sync(true);
                for (i, v) in [editor.cursor.0, editor.cursor.1].into_iter().enumerate() {
                    editor.cursor_at[i].jump(v);
                }
                editor.kept = editor.edits.clone();
                shell.show(Box::new(editor));
                crate::fullheight::settle(&mut shell, 90);
                shell.draw();
                let file = format!("{name}-{}-{}.png", if light { "light" } else { "dark" }, size.name());
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(file), 2);
                sheets[n].push(shell.gfx.canvas.clone());
            }
        }
    }
    for ((name, _), shots) in pages.iter().zip(&sheets) {
        crate::platform::sim::save_png(&sheet(shots), &dir.join(format!("sheet-{name}.png")), 2);
    }
    // The marked-up edit exported at full size, as it is written.
    let mut e = super::edits::Edits { look: Look::Contemporary, marks: marks(), ..Default::default() };
    e.set(Adjust::Contrast, 20);
    let src = crate::photos::decode(&photo, 1640, 1232).expect("the photo");
    let out = export::render(&src, &e, super::cores(), &|_| {});
    crate::camera::save_jpeg(&dir.join("editor-export-full-size.jpg"), &out).ok();
    std::fs::remove_dir_all(&state).ok();
    println!("the editor's pages in {}", dir.display());
}

/// Six shots of a page on one sheet: dark above light, compact to large.
pub fn sheet(shots: &[bezel::Canvas]) -> bezel::Canvas {
    let (w, h) = (crate::gfx::W as i32, crate::gfx::H as i32);
    let gap = 6;
    let mut sheet = bezel::Canvas::new(3 * w + 2 * gap, 2 * h + gap);
    sheet.px.fill(0x0080_8080);
    for (i, shot) in shots.iter().enumerate() {
        let (ox, oy) = ((i % 3) as i32 * (w + gap), (i / 3) as i32 * (h + gap));
        for y in 0..h {
            let from = (y * shot.w) as usize;
            let to = ((oy + y) * sheet.w + ox) as usize;
            sheet.px[to..to + w as usize].copy_from_slice(&shot.px[from..from + w as usize]);
        }
    }
    sheet
}
