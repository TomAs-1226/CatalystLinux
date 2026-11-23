use super::enhance::{self, Enhance};
use super::quad::{self, Quad};
use super::warp::{self, Homography, Paper};
use super::*;
use crate::photos::Picture;

/// A table: a dark, uneven wood-ish grey with grain and noise.
fn table(w: usize, h: usize) -> Picture {
    let mut pic = Picture::new(w, h);
    let mut seed = 0x9E37_79B9u32;
    for y in 0..h {
        for x in 0..w {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let grain = ((x as f32 * 0.21 + (y as f32 * 0.05).sin() * 6.0).sin() * 9.0) as i32;
            let v = 70 + (y * 30 / h) as i32 + grain + (seed % 13) as i32 - 6;
            let at = (y * w + x) * 3;
            pic.rgb[at..at + 3].copy_from_slice(&[(v + 12).clamp(0, 255) as u8, (v + 4).clamp(0, 255) as u8, v.clamp(0, 255) as u8]);
        }
    }
    pic
}

fn luma(pic: &Picture) -> Vec<u8> {
    pic.rgb.chunks_exact(3).map(|p| ((77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2])) >> 8) as u8).collect()
}

fn worst(a: &Quad, b: &Quad) -> f32 {
    quad::moved(a, b)
}

#[test]
fn a_page_at_an_angle_on_a_table_is_found_by_its_corners() {
    for truth in [
        [(78.0, 30.0), (238.0, 46.0), (226.0, 214.0), (62.0, 196.0)],
        [(120.0, 20.0), (270.0, 70.0), (205.0, 225.0), (52.0, 170.0)],
        // Seen at a slant: the far edge shorter.
        [(105.0, 40.0), (215.0, 40.0), (250.0, 220.0), (70.0, 220.0)],
    ] {
        let mut pic = table(320, 240);
        super::sim::paint_at(&mut pic, &truth, 0.95, 1.0);
        let found = quad::find(&luma(&pic), 320, 240).expect("the page is found");
        assert!(worst(&found, &truth) < 2.5, "found {found:?}, truth {truth:?}");
    }
    // An empty table: no page.
    assert!(quad::find(&luma(&table(320, 240)), 320, 240).is_none());
    // A sliver is not a page.
    let mut pic = table(320, 240);
    super::sim::paint_at(&mut pic, &[(20.0, 100.0), (300.0, 104.0), (300.0, 118.0), (20.0, 114.0)], 0.95, 1.0);
    assert!(quad::find(&luma(&pic), 320, 240).is_none());
}

#[test]
fn the_largest_quad_of_a_hull_is_its_four_extreme_corners() {
    // An octagon-ish hull around a square: the square's corners win.
    let hull = quad::hull(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (5.0, 0.2), (9.8, 5.0), (5.0, 9.8), (0.2, 5.0), (5.0, 5.0)]);
    let q = quad::order(quad::largest_quad(&hull).unwrap());
    assert_eq!(q, [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    assert!((quad::area(&q) - 100.0).abs() < 1e-3);
}

#[test]
fn the_sim_page_in_the_cameras_view_is_found_and_held() {
    // The simulated camera's own preview: the scene, the page laid on it.
    let mut pic = crate::camera::scene(320, 240, 4.0, false);
    super::sim::paint(&mut pic, [0.0, 0.0, 1.0, 1.0], 4.0);
    let truth = super::sim::corners(320.0, 240.0, 4.0);
    let found = quad::find(&luma(&pic), 320, 240).expect("found");
    assert!(worst(&found, &truth) < 3.0, "found {found:?}, truth {truth:?}");
}

#[test]
fn the_page_is_flattened_back_to_what_it_was() {
    let sheet = super::sim::sheet();
    // The sheet seen at a slant on a table, then drawn flat again.
    let truth = [(300.0, 120.0), (900.0, 170.0), (980.0, 900.0), (240.0, 860.0)];
    let mut photo = table(1200, 1000);
    super::sim::paint_at(&mut photo, &truth, 1.0, 0.0);
    let flat = warp::flatten(&photo, &truth, 420, 594, 2);
    let want = sheet.cover(420, 594);
    let off: u64 = flat.rgb.iter().zip(&want.rgb).map(|(a, b)| u64::from(a.abs_diff(*b))).sum::<u64>() / flat.rgb.len() as u64;
    // Evenly lit: what comes back is the sheet, its text and lines where they were.
    assert!(off < 8, "mean difference {off}");
    let lum = |p: &Picture, x: usize, y: usize| p.rgb[(y * p.w + x) * 3];
    // The heading's rule (orange, at a sixth of the way down) is a sixth of the way down.
    let rule_y = (206.0 / 1188.0 * 594.0) as usize;
    assert!(lum(&flat, 210, rule_y) > 150 && flat.rgb[(rule_y * 420 + 210) * 3 + 2] < 110, "{:?}", &flat.rgb[(rule_y * 420 + 210) * 3..(rule_y * 420 + 210) * 3 + 3]);
    // The homography takes each corner exactly.
    let h = Homography::between([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], truth).unwrap();
    for (i, (x, y)) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].iter().enumerate() {
        let (u, v) = h.at(*x, *y);
        assert!((u - f64::from(truth[i].0)).abs() < 1e-6 && (v - f64::from(truth[i].1)).abs() < 1e-6);
    }
}

#[test]
fn the_pages_true_shape_comes_back_through_the_lens() {
    // An A4 sheet tipped 24° away and turned: its corners in the photo are
    // far from A4's shape, yet the lens gives its own back.
    let (w, h) = (3280.0, 2464.0);
    let q = super::sim::corners(w, h, 0.0);
    let ratio = warp::page_ratio(&q, (w, h), warp::FOCAL_FULL);
    assert!((ratio / 1.41421 - 1.0).abs() < 0.02, "ratio {ratio}");
    let naive = (quad::area(&q)).sqrt();
    assert!(naive > 0.0);
    assert_eq!(Paper::Auto.resolve(ratio), Paper::A4);
    assert_eq!(Paper::Auto.resolve(11.0 / 8.5 * 1.01), Paper::Letter);
    assert_eq!(Paper::Auto.resolve(1.6), Paper::Free);
    assert_eq!(Paper::Letter.resolve(1.414), Paper::Letter);
    // Flattened at about the pixels the photo gives it, long side capped.
    let (fw, fh) = warp::flat_size(&q, ratio, 2200);
    assert!((fh as f32 / fw as f32 - ratio).abs() < 0.01 && fh <= 2200, "{fw} × {fh}");
}

#[test]
fn black_and_white_keeps_text_whole_across_a_shadow() {
    // Paper lit from 250 on the left to 60 on the right, with ink squares
    // at a quarter of the light: the left's ink is brighter than the right's
    // paper, so no one threshold would do.
    let (w, h) = (400, 200);
    let mut page = Picture::new(w, h);
    let light = |x: usize| 250.0 - 190.0 * x as f32 / (w - 1) as f32;
    let ink = |x: usize, y: usize| (x % 50) >= 20 && (x % 50) < 32 && (60..140).contains(&y);
    for y in 0..h {
        for x in 0..w {
            let v = if ink(x, y) { light(x) * 0.25 } else { light(x) };
            page.rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].fill(v as u8);
        }
    }
    assert!(light(10) * 0.25 > light(w - 1), "the test's own premise");
    let out = enhance::enhance(&page, Enhance::Mono);
    let mut wrong = 0;
    for y in 0..h {
        for x in 0..w {
            let v = out.rgb[(y * w + x) * 3];
            assert!(v == 0 || v == 255);
            if (v == 0) != ink(x, y) {
                wrong += 1;
            }
        }
    }
    assert!(wrong * 1000 < w * h * 5, "{wrong} pixels wrong of {}", w * h);
    // Grey and colour: the paper comes out white on both sides of the shadow.
    for how in [Enhance::Grey, Enhance::Colour] {
        let out = enhance::enhance(&page, how);
        for x in [5, w / 2, w - 5] {
            assert!(out.rgb[(10 * w + x) * 3] > 235, "{:?} at {x}: {}", how, out.rgb[(10 * w + x) * 3]);
        }
        assert!(out.rgb[(100 * w + 25) * 3] < 40, "{:?}: the ink", how);
    }
}

#[test]
fn the_pdf_has_its_pages_and_a_true_cross_reference() {
    let colour = Picture { w: 8, h: 10, rgb: vec![200; 240] };
    let pages = vec![
        pdf::Page { size: (595.28, 841.89), image: pdf::Image::Jpeg { data: jpeg_bytes(&colour, 85, false).unwrap(), w: 8, h: 10, grey: false } },
        pdf::Page { size: (612.0, 792.0), image: pdf::Image::Jpeg { data: jpeg_bytes(&colour, 85, true).unwrap(), w: 8, h: 10, grey: true } },
        pdf::Page { size: (595.28, 400.0), image: pdf::Image::Bits { rows: pdf::pack_bits(&[true, false, true, true, false, false, true, true, true, false], 10, 1), w: 10, h: 1 } },
    ];
    let bytes = pdf::write(&pages, "Scan (test)");
    // Read as bytes: the pictures in it are not text.
    let text: String = bytes.iter().map(|b| if b.is_ascii() { *b as char } else { '\u{1}' }).collect();
    assert!(text.starts_with("%PDF-1.4\n"));
    assert!(text.trim_end().ends_with("%%EOF"));
    // startxref points at the table; each entry at its object.
    let at = text.rfind("startxref\n").unwrap();
    let xref: usize = text[at + 10..].lines().next().unwrap().trim().parse().unwrap();
    assert!(text[xref..].starts_with("xref\n0 "));
    let mut lines = text[xref..].split('\n');
    lines.next();
    let count: usize = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    assert_eq!(count, 3 + 3 * 3 + 1);
    let free = lines.next().unwrap();
    assert_eq!(free, "0000000000 65535 f ");
    for n in 1..count {
        let line = lines.next().unwrap();
        assert_eq!(line.len(), 19, "each entry is twenty bytes with its newline");
        let offset: usize = line[..10].parse().unwrap();
        assert!(text[offset..].starts_with(&format!("{n} 0 obj\n")), "object {n} at {offset}");
    }
    assert!(text.contains(&format!("trailer\n<< /Size {count} /Root 1 0 R")));
    assert!(text.contains("/Count 3 >>"));
    assert_eq!(text.matches("/Type /Page /Parent").count(), 3);
    assert_eq!(text.matches("/Filter /DCTDecode").count(), 2);
    assert!(text.contains("/BitsPerComponent 1 /Filter /RunLengthDecode"));
    // Run-length coding reads back as it was.
    for data in [vec![], vec![7], vec![1, 2, 3, 3, 3, 3, 4], vec![255; 300], (0..=255u8).chain([9; 200]).chain(0..40).collect::<Vec<u8>>()] {
        assert_eq!(pdf::run_length_back(&pdf::run_length(&data)), data);
    }
    assert!(pdf::run_length(&[255; 1000]).len() < 20);
    assert_eq!(pdf::pack_bits(&[true, false, true, true, false, false, true, true, true], 9, 1), vec![0b1011_0011, 0b1000_0000]);
}

#[test]
fn a_document_saves_as_a_pdf_and_pngs_never_over_another() {
    let dir = std::env::temp_dir().join(format!("catalyst-scans-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join(".work")).unwrap();
    let page = Picture { w: 60, h: 85, rgb: vec![230; 60 * 85 * 3] };
    let flat = dir.join(".work").join("p1.jpg");
    std::fs::write(&flat, jpeg_bytes(&page, 92, false).unwrap()).unwrap();
    let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 22).unwrap().and_hms_opt(14, 15, 0).unwrap();
    // A scan of that second already there.
    std::fs::write(dir.join("SCAN_20261122_141500.pdf"), b"kept").unwrap();
    let pages = vec![(flat.clone(), Enhance::Mono, 1.4142, Paper::A4, 1.41), (flat.clone(), Enhance::Colour, 1.4142, Paper::Free, 1.41)];
    let pdf_path = save_pages(&dir, &pages, now, &|_| {}).unwrap();
    assert_eq!(pdf_path, dir.join("SCAN_20261122_141500_2.pdf"));
    assert_eq!(std::fs::read(dir.join("SCAN_20261122_141500.pdf")).unwrap(), b"kept");
    let text = String::from_utf8_lossy(&std::fs::read(&pdf_path).unwrap()).into_owned();
    assert!(text.contains("/Count 2 >>") && text.contains("/MediaBox [0 0 595.28 841.89]"));
    for n in 1..=2 {
        let png = dir.join(format!("SCAN_20261122_141500_2_{n}.png"));
        let reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&png).unwrap())).read_info().unwrap();
        assert_eq!(reader.info().width, 60);
        assert_eq!(reader.info().height, 85);
        assert_eq!(reader.info().bit_depth, if n == 1 { png::BitDepth::One } else { png::BitDepth::Eight });
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_finder_looks_on_its_own_thread_a_few_times_a_second() {
    let mut finder = Finder::new();
    let mut pic = table(320, 240);
    let truth = [(78.0, 30.0), (238.0, 46.0), (226.0, 214.0), (62.0, 196.0)];
    super::sim::paint_at(&mut pic, &truth, 0.95, 1.0);
    assert!(finder.wants());
    let began = std::time::Instant::now();
    finder.give(&luma(&pic), 320, 240);
    assert!(!finder.wants(), "one frame at a time, a fifth of a second apart");
    let found = loop {
        if let Some(found) = finder.take() {
            break found;
        }
        assert!(began.elapsed().as_secs() < 5);
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    let took = began.elapsed().as_secs_f64() * 1000.0;
    eprintln!("the page found in {took:.2} ms here; about {:.0} ms reckoned on the handheld", took * 15.0);
    assert!(worst(&found.unwrap(), &truth) < 2.5);
    // Timed alone: the finding itself.
    let y = luma(&pic);
    let began = std::time::Instant::now();
    for _ in 0..10 {
        quad::find(&y, 320, 240);
    }
    let each = began.elapsed().as_secs_f64() * 1000.0 / 10.0;
    eprintln!("quad::find on a 320 × 240 frame: {each:.2} ms here");
    assert!(each < 12.0, "{each} ms");
}
