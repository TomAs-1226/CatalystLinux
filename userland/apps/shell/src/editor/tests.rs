use super::develop::{self, Tone};
use super::edits::{self, Adjust, Aspect, Edits, Ink, Mark, Shape, WHOLE};
use super::*;
use crate::photos::Picture;

/// One pixel through the adjustments of `e`.
fn through(e: &Edits, p: [u8; 3]) -> [u8; 3] {
    Tone::of(e).pixel(p)
}

fn with(a: Adjust, v: i32) -> Edits {
    let mut e = Edits::default();
    e.set(a, v);
    e
}

fn near(got: [u8; 3], want: [u8; 3], what: &str) {
    for c in 0..3 {
        assert!((i32::from(got[c]) - i32::from(want[c])).abs() <= 1, "{what}: {got:?}, wanted {want:?}");
    }
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("catalyst-editor-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn flat(w: usize, h: usize, rgb: [u8; 3]) -> Picture {
    Picture { w, h, rgb: rgb.repeat(w * h) }
}

#[test]
fn each_adjustment_takes_a_known_pixel_where_it_should() {
    // Nothing: as it was.
    near(through(&Edits::default(), [12, 128, 240]), [12, 128, 240], "nothing");
    // A stop brighter, on linear light: 0.502 → 0.688.
    near(through(&with(Adjust::Exposure, 50), [128; 3]), [175; 3], "exposure +1 stop");
    near(through(&with(Adjust::Exposure, -50), [128; 3]), [93; 3], "exposure −1 stop");
    // Contrast as an S: the quarter darkens to smoothstep's, the middle stays.
    near(through(&with(Adjust::Contrast, 100), [64; 3]), [40; 3], "contrast +100 at a quarter");
    near(through(&with(Adjust::Contrast, 100), [128; 3]), [128; 3], "contrast +100 at the middle");
    // Flattened toward grey: black halfway up to the middle.
    near(through(&with(Adjust::Contrast, -100), [0; 3]), [64; 3], "contrast −100 at black");
    // Highlights pull the three-quarter tone down a fifth; shadows lift the quarter one.
    near(through(&with(Adjust::Highlights, -100), [191; 3]), [140; 3], "highlights −100");
    near(through(&with(Adjust::Shadows, 100), [64; 3]), [115; 3], "shadows +100");
    // Saturation: none is the pixel's grey; double pushes away from it.
    near(through(&with(Adjust::Saturation, -100), [200, 100, 50]), [124; 3], "saturation −100");
    near(through(&with(Adjust::Saturation, 100), [200, 100, 50]), [255, 76, 0], "saturation +100");
    // Warmth: red up a fifth, blue down; tint: green down 15%.
    near(through(&with(Adjust::Warmth, 100), [128; 3]), [154, 128, 102], "warmth +100");
    near(through(&with(Adjust::Tint, 100), [128; 3]), [128, 109, 128], "tint +100");
    // Vibrance lifts the dull and leaves the vivid (and the grey) be.
    near(through(&with(Adjust::Vibrance, 100), [160, 128, 96]), [179, 124, 68], "vibrance on a dull colour");
    near(through(&with(Adjust::Vibrance, 100), [255, 0, 0]), [255, 0, 0], "vibrance on a vivid one");
    near(through(&with(Adjust::Vibrance, 100), [128; 3]), [128; 3], "vibrance on grey");
    // A look after the fact: Mono's grey, whatever comes in.
    let mono = Edits { look: crate::filters::Look::Mono, ..Edits::default() };
    let p = through(&mono, [200, 40, 40]);
    assert!(p[0] == p[1] && p[1] == p[2], "mono: {p:?}");

    // Sharpness: flat stays flat; an edge gains its overshoot either side.
    let e = with(Adjust::Sharpness, 100);
    let tone = Tone::of(&e);
    assert_eq!(develop::develop(flat(6, 3, [120; 3]), &tone, 1).rgb, flat(6, 3, [120; 3]).rgb);
    let mut edge = flat(6, 3, [100; 3]);
    for y in 0..3 {
        for x in 3..6 {
            edge.rgb[(y * 6 + x) * 3..(y * 6 + x) * 3 + 3].fill(200);
        }
    }
    let sharp = develop::develop(edge, &tone, 1);
    let at = |x: usize| sharp.rgb[(6 + x) * 3];
    assert!(at(2) < 80 && at(3) > 220 && at(0) == 100 && at(5) == 200, "{:?}", (0..6).map(at).collect::<Vec<_>>());

    // The vignette: the middle untouched, a corner nearly three quarters darker.
    let e = with(Adjust::Vignette, -100);
    let out = develop::develop(flat(100, 100, [200; 3]), &Tone::of(&e), 1);
    assert_eq!(out.rgb[(50 * 100 + 50) * 3], 200);
    let corner = out.rgb[0];
    assert!((49..=51).contains(&corner), "corner {corner}");
    let e = with(Adjust::Vignette, 100);
    let out = develop::develop(flat(100, 100, [100; 3]), &Tone::of(&e), 1);
    assert!(out.rgb[0] > 180, "a light vignette lifts the corner: {}", out.rgb[0]);
}

#[test]
fn the_tone_curve_never_turns_back_on_itself() {
    for a in [Adjust::Exposure, Adjust::Contrast, Adjust::Highlights, Adjust::Shadows] {
        for v in [-100, -50, 50, 100] {
            let e = with(a, v);
            let mut last = -1.0;
            for i in 0..=255 {
                let y = develop::tone(i as f32 / 255.0, &e);
                assert!(y >= last - 1e-6, "{} {v} at {i}", a.name());
                last = y;
            }
        }
    }
}

#[test]
fn the_geometry_maps_corners_where_turns_flips_and_crops_put_them() {
    let e = Edits::default();
    let m = develop::mapping((400, 300), &e, WHOLE, (400, 300));
    assert_eq!(m.at(0.0, 0.0), (0.0, 0.0));
    assert_eq!(m.at(400.0, 300.0), (400.0, 300.0));
    // A quarter turn clockwise: the made picture's top-left was the photo's bottom-left.
    let turned = Edits { turns: 1, ..Edits::default() };
    let m = develop::mapping((400, 300), &turned, WHOLE, (300, 400));
    let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
    assert!(close(m.at(0.0, 0.0), (0.0, 300.0)), "{:?}", m.at(0.0, 0.0));
    assert!(close(m.at(300.0, 0.0), (0.0, 0.0)));
    assert!(close(m.at(300.0, 400.0), (400.0, 0.0)));
    let three = Edits { turns: 3, ..Edits::default() };
    let m = develop::mapping((400, 300), &three, WHOLE, (300, 400));
    assert!(close(m.at(0.0, 0.0), (400.0, 0.0)));
    // Mirrored: the top-left from the top-right.
    let mirror = Edits { flip_h: true, ..Edits::default() };
    assert!(close(develop::mapping((400, 300), &mirror, WHOLE, (400, 300)).at(0.0, 0.0), (400.0, 0.0)));
    // A crop: its corners, at any size.
    let cropped = Edits { crop: [0.25, 0.5, 0.5, 0.5], ..Edits::default() };
    let m = develop::mapping((400, 300), &cropped, cropped.crop, (200, 150));
    assert!(close(m.at(0.0, 0.0), (100.0, 150.0)));
    assert!(close(m.at(200.0, 150.0), (300.0, 300.0)));
    // Straightening enlarges just enough that no corner is empty.
    assert!((develop::fill_scale(4.0, 3.0, 0.0) - 1.0).abs() < 1e-6);
    assert!((develop::fill_scale(400.0, 300.0, 10.0) - 1.21634).abs() < 1e-4);
    for deg in [-15.0, -7.5, 3.0, 15.0] {
        let e = Edits { straighten: deg, ..Edits::default() };
        let m = develop::mapping((400, 300), &e, WHOLE, (400, 300));
        for (x, y) in [(0.0, 0.0), (400.0, 0.0), (400.0, 300.0), (0.0, 300.0)] {
            let (sx, sy) = m.at(x, y);
            assert!((-0.01..=400.01).contains(&sx) && (-0.01..=300.01).contains(&sy), "{deg}°: corner ({x}, {y}) from ({sx}, {sy})");
        }
        // And one of them touches the picture's edge: no more than needed.
        let gap = [(0.0, 0.0), (400.0, 0.0), (400.0, 300.0), (0.0, 300.0)]
            .iter()
            .map(|(x, y)| {
                let (sx, sy) = m.at(*x, *y);
                sx.min(400.0 - sx).min(sy).min(300.0 - sy)
            })
            .fold(f32::MAX, f32::min);
        assert!(gap < 0.01, "{deg}°: {gap} px from the edge at the nearest");
    }
    // The pixels follow: a red corner turned clockwise lands top-right.
    let mut pic = flat(4, 2, [0; 3]);
    pic.rgb[0] = 255;
    let out = develop::shape(&pic, &turned, false, (2, 4), 1);
    assert_eq!((out.w, out.h), (2, 4));
    assert!(out.rgb[3] > 120 && out.rgb[0] < 120, "{:?}", &out.rgb[..6]);
}

#[test]
fn crop_shapes_are_the_largest_of_their_aspect_and_turn_with_the_picture() {
    let b = aspect_box(Aspect::SixteenNine, (4000.0, 3000.0), (0.5, 0.5), 1.0);
    assert!((b[2] - 1.0).abs() < 1e-5 && (b[3] - 0.75).abs() < 1e-5 && (b[1] - 0.125).abs() < 1e-5, "{b:?}");
    // On an upright picture, 4:3 is 3:4.
    let b = aspect_box(Aspect::FourThree, (3000.0, 4000.0), (0.5, 0.5), 1.0);
    assert!((b[2] - 1.0).abs() < 1e-5 && (b[3] - 1.0).abs() < 1e-5, "{b:?}");
    let b = aspect_box(Aspect::Square, (4000.0, 3000.0), (0.9, 0.5), 0.5);
    assert!((b[2] * 4000.0 - b[3] * 3000.0).abs() < 0.5 && b[0] + b[2] <= 1.0 + 1e-6, "{b:?}");
    assert_eq!(aspect_box(Aspect::Free, (4.0, 3.0), (0.5, 0.5), 1.0), WHOLE);
    // A crop turns with its picture: the left strip becomes the top one.
    let dir = scratch("turn");
    let photo = dir.join("IMG_20261121_101500.jpg");
    std::fs::write(&photo, crate::docscan::jpeg_bytes(&flat(64, 48, [90, 120, 150]), 90, false).unwrap()).unwrap();
    let mut ed = Editor::open_now(photo);
    ed.edits.crop = [0.0, 0.0, 0.25, 1.0];
    ed.turn(1);
    let c = ed.edits.crop;
    assert!(c[0].abs() < 1e-6 && c[1].abs() < 1e-6 && (c[2] - 1.0).abs() < 1e-6 && (c[3] - 0.25).abs() < 1e-6, "{c:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_edit_survives_its_sidecar() {
    let mut e = Edits { look: crate::filters::Look::Tint, hue: 210.0, turns: 3, flip_h: true, flip_v: false, straighten: -4.5, aspect: Aspect::ThreeTwo, crop: [0.1, 0.05, 0.75, 0.5], ..Edits::default() };
    for (i, a) in super::edits::ADJUSTS.iter().enumerate() {
        e.set(*a, (i as i32 * 15) - 60);
    }
    e.marks.push(Mark { shape: Shape::Arrow, ink: Ink::Signal, a: (0.2, 0.3), b: (0.55, 0.6), text: String::new() });
    e.marks.push(Mark { shape: Shape::Text, ink: Ink::Ice, a: (0.5, 0.25), b: (0.5, 0.25), text: "Intake, left side".into() });
    e.marks.push(Mark { shape: Shape::Circle, ink: Ink::Ice, a: (0.1, 0.1), b: (0.3, 0.2), text: String::new() });
    assert_eq!(Edits::from_text(&e.to_text()), e);
    let dir = scratch("sidecar");
    let photo = dir.join("IMG_20261121_101500.jpg");
    std::fs::write(&photo, b"not read").unwrap();
    edits::save(&photo, &e).unwrap();
    assert!(edits::has_edit(&photo));
    assert_eq!(edits::load(&photo), e);
    // Small: a few hundred bytes.
    assert!(std::fs::metadata(edits::sidecar(&photo)).unwrap().len() < 600);
    // Back to nothing, the sidecar goes; the photo was never touched.
    edits::save(&photo, &Edits::default()).unwrap();
    assert!(!edits::has_edit(&photo));
    assert_eq!(std::fs::read(&photo).unwrap(), b"not read");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_export_is_a_new_photo_beside_the_original_and_never_overwrites() {
    let dir = scratch("export");
    let photo = dir.join("IMG_20261121_101500.jpg");
    // Red in the bottom-left quarter: turned a quarter clockwise, the top-left.
    let mut pic = flat(160, 120, [90, 120, 150]);
    for y in 60..120 {
        for x in 0..80 {
            pic.rgb[(y * 160 + x) * 3..(y * 160 + x) * 3 + 3].copy_from_slice(&[220, 60, 40]);
        }
    }
    let original = crate::docscan::jpeg_bytes(&pic, 92, false).unwrap();
    std::fs::write(&photo, &original).unwrap();
    // An earlier export, kept.
    let earlier = dir.join("IMG_20261121_101500_edit.jpg");
    std::fs::write(&earlier, b"an earlier edit").unwrap();
    assert_eq!(super::export::edit_path(&photo), dir.join("IMG_20261121_101500_edit_2.jpg"));
    let mut e = Edits { crop: [0.0, 0.0, 0.5, 0.5], turns: 1, ..Edits::default() };
    e.set(Adjust::Exposure, 25);
    e.marks.push(Mark { shape: Shape::Box, ink: Ink::Signal, a: (0.1, 0.1), b: (0.9, 0.9), text: String::new() });
    let job = super::export::start(photo.clone(), e);
    let began = std::time::Instant::now();
    let made = loop {
        if let Some(done) = job.done() {
            break done.unwrap();
        }
        assert!(began.elapsed().as_secs() < 30, "the export never finished");
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(made, dir.join("IMG_20261121_101500_edit_2.jpg"));
    assert_eq!(std::fs::read(&photo).unwrap(), original, "the original is never written");
    assert_eq!(std::fs::read(&earlier).unwrap(), b"an earlier edit");
    // The crop of the red quarter, turned: 60 × 80, red, the box's ring in signal.
    let out = crate::photos::decode(&made, 1000, 1000).unwrap();
    assert_eq!((out.w, out.h), (60, 80));
    let mid = &out.rgb[(40 * 60 + 30) * 3..(40 * 60 + 30) * 3 + 3];
    assert!(mid[0] > 200 && mid[1] < 120, "{mid:?}");
    assert!(job.fraction() >= 0.999);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn marks_are_drawn_into_the_export_as_on_the_panel() {
    let mut pic = flat(400, 300, [0; 3]);
    let (signal, _) = super::markup::colours(Ink::Signal);
    let (ice, on_ice) = super::markup::colours(Ink::Ice);
    let marks = vec![
        Mark { shape: Shape::Arrow, ink: Ink::Signal, a: (0.1, 0.5), b: (0.6, 0.5), text: String::new() },
        Mark { shape: Shape::Text, ink: Ink::Ice, a: (0.5, 0.15), b: (0.5, 0.15), text: "Bumper".into() },
    ];
    super::markup::draw(&mut pic, &marks);
    let at = |x: usize, y: usize| [pic.rgb[(y * 400 + x) * 3], pic.rgb[(y * 400 + x) * 3 + 1], pic.rgb[(y * 400 + x) * 3 + 2]];
    // Along the shaft, the signal colour; well off it, nothing.
    assert_eq!(at(120, 150), [signal.0, signal.1, signal.2]);
    assert_eq!(at(120, 170), [0, 0, 0]);
    // The head reaches back from its point, either side of the shaft.
    let head = super::markup::paths(&marks[0], 400.0, 300.0);
    assert_eq!(head.len(), 2);
    assert!(head[1][0].1 < 150.0 && head[1][2].1 > 150.0);
    // The label: its capsule in ice, its words in the colour for words on ice.
    let [x, y, w, h] = super::markup::label_box(&marks[1], 400.0, 300.0, 50.0);
    assert!(x > 0.0 && y >= 0.0 && x + w <= 400.0 && y + h <= 300.0);
    let edge = at((x + 4.0 + h * 0.5) as usize, (y + 3.0) as usize);
    assert_eq!(edge, [ice.0, ice.1, ice.2]);
    let dist = |p: [u8; 3], c: bezel::Color| (i32::from(p[0]) - i32::from(c.0)).abs() + (i32::from(p[1]) - i32::from(c.1)).abs() + (i32::from(p[2]) - i32::from(c.2)).abs();
    let inked = (y as usize..(y + h) as usize).flat_map(|yy| (x as usize..(x + w) as usize).map(move |xx| (xx, yy))).filter(|(xx, yy)| dist(at(*xx, *yy), on_ice) < 60).count();
    assert!(inked > 20, "the label's words: {inked} pixels");
}

#[test]
fn the_preview_retones_within_a_frame() {
    // A photo's working copy, and the preview at the panel's view size,
    // every adjustment, a look, the vignette and sharpening on: timed here,
    // the A53 reckoned fifteen times slower.
    let dir = scratch("timing");
    let photo = dir.join("IMG_20261121_101500.jpg");
    std::fs::write(&photo, crate::docscan::jpeg_bytes(&crate::camera::scene(1640, 1232, 3.0, false), 90, false).unwrap()).unwrap();
    let mut ed = Editor::open_now(photo);
    for (i, a) in super::edits::ADJUSTS.iter().enumerate() {
        ed.edits.set(*a, 20 + i as i32 * 3);
    }
    ed.edits.look = crate::filters::Look::Classic;
    ed.refresh((161.0, 121.0));
    assert_eq!((ed.shown.w, ed.shown.h), (161, 121));
    ed.retone();
    let began = std::time::Instant::now();
    for k in 0..40 {
        ed.edits.set(Adjust::Exposure, k);
        ed.retone();
    }
    let each = began.elapsed().as_secs_f64() * 1000.0 / 40.0;
    eprintln!("the editor's preview, retoned: {each:.3} ms here; about {:.1} ms reckoned on the handheld", each * 15.0);
    assert!(each < 0.8, "{each} ms a change");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn keys_change_rows_and_leave_the_photo_alone() {
    crate::shell::tests::sandbox();
    let dir = scratch("keys");
    let photo = dir.join("IMG_20261121_101500.jpg");
    let bytes = crate::docscan::jpeg_bytes(&flat(64, 48, [90, 120, 150]), 90, false).unwrap();
    std::fs::write(&photo, &bytes).unwrap();
    let mut ed = Editor::open_now(photo.clone());
    let mut robot = crate::robot::Robot::none();
    let mut settings = crate::shell::Settings::default();
    let mut say = None;
    let mut press = |ed: &mut Editor, k: Key| ed.key(k, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
    press(&mut ed, Key::Right);
    press(&mut ed, Key::Right);
    assert_eq!(ed.edits.get(Adjust::Exposure), 10);
    press(&mut ed, Key::Backspace);
    assert_eq!(ed.edits.get(Adjust::Exposure), 0);
    // The looks: ↓ lays the next on.
    press(&mut ed, Key::Tab);
    press(&mut ed, Key::Down);
    assert_eq!(ed.edits.look, crate::filters::Look::Vivid);
    // Markup: a box from one place to another.
    press(&mut ed, Key::Char('4'));
    press(&mut ed, Key::Right);
    assert_eq!(ed.tool, Shape::Box);
    press(&mut ed, Key::Enter);
    press(&mut ed, Key::Enter);
    press(&mut ed, Key::Right);
    press(&mut ed, Key::Down);
    press(&mut ed, Key::Enter);
    assert_eq!(ed.edits.marks.len(), 1);
    assert_eq!(ed.edits.marks[0].shape, Shape::Box);
    // A label is typed in (box, arrow, then round to text).
    press(&mut ed, Key::Left);
    press(&mut ed, Key::Left);
    assert_eq!(ed.tool, Shape::Text);
    press(&mut ed, Key::Enter);
    press(&mut ed, Key::Enter);
    assert!(ed.wants_text());
    for c in "Gear".chars() {
        press(&mut ed, Key::Char(c));
    }
    press(&mut ed, Key::Enter);
    assert_eq!(ed.edits.marks.last().map(|m| m.text.as_str()), Some("Gear"));
    assert!(matches!(press(&mut ed, Key::Back), Nav::Pop));
    assert!(edits::has_edit(&photo));
    assert_eq!(std::fs::read(&photo).unwrap(), bytes);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn s_exports_and_each_states_keys_fit_the_sheet() {
    crate::shell::tests::sandbox();
    let dir = scratch("sheet");
    let photo = dir.join("IMG_20261121_101500.jpg");
    std::fs::write(&photo, crate::docscan::jpeg_bytes(&flat(64, 48, [90, 120, 150]), 90, false).unwrap()).unwrap();
    let mut ed = Editor::open_now(photo);
    let mut robot = crate::robot::Robot::none();
    let mut settings = crate::shell::Settings::default();
    let mut say = None;
    let mut g = crate::gfx::Gfx::new(bezel::Tokens::dark());
    let mut fits = |ed: &Editor| crate::keysheet::fits(&mut g, &ed.key_sheet()).unwrap();
    for (panel, rows) in [(Panel::Adjust, 1), (Panel::Looks, 1), (Panel::Geometry, 4), (Panel::Markup, 1)] {
        ed.set_panel(panel);
        for row in 0..rows {
            ed.lists[panel.index()].focus = row;
            fits(&ed);
        }
    }
    ed.edits.look = crate::filters::Look::Tint;
    ed.set_panel(Panel::Looks);
    fits(&ed);
    for (tool, aspect) in [(CropTool::Move, Aspect::Free), (CropTool::Size, Aspect::Free), (CropTool::Size, edits::ASPECTS[1])] {
        ed.edits.aspect = aspect;
        ed.crop_tool = Some(tool);
        fits(&ed);
    }
    ed.crop_tool = None;
    for placing in [Placing::Start, Placing::End((0.2, 0.2)), Placing::Typing((0.2, 0.2), String::new())] {
        ed.placing = Some(placing);
        fits(&ed);
    }
    ed.placing = None;
    ed.before = true;
    fits(&ed);
    ed.before = false;
    // S exports (a bare X is the down arrow outside typing).
    ed.edits.set(Adjust::Exposure, 10);
    ed.key(Key::Char('s'), &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
    assert!(ed.export.is_some(), "s starts the export");
    let began = std::time::Instant::now();
    while ed.export.is_some() {
        assert!(began.elapsed().as_secs() < 30, "the export never finished");
        ed.tick(0.05);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::fs::remove_dir_all(&dir).ok();
}
