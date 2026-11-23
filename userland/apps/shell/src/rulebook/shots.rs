//! `catalyst-shell --rulebook-shots <dir>`: Rulebook's pages, from the
//! sample manual (never FIRST's text), in Bezel's look, dark and light, at
//! each text size: each page at twice the panel's size in
//! `<dir>/<dark|light>-<size>/`, and a sheet of all six as
//! `<dir>/sheet-<page>.png` (compact, standard, large across; dark, then
//! light, down).

use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::ask::Who;
use super::store::{Book, Meta};
use super::{fetch, sample, store, Asking, Entries, ManualPage, Reader, Rulebook, Searching};
use crate::fullheight::settle;
use crate::gfx::TextSize;
use crate::platform::Key;
use crate::shell::Shell;

fn press(shell: &mut Shell, keys: &[Key]) {
    for key in keys {
        shell.key(*key);
        settle(shell, 30);
    }
    settle(shell, 50);
}

/// Frames for `ms` of real time (an answer on its thread).
fn pause(shell: &mut Shell, ms: u64) {
    let began = Instant::now();
    while began.elapsed() < Duration::from_millis(ms) {
        std::thread::sleep(Duration::from_millis(5));
        settle(shell, 2);
    }
    settle(shell, 60);
}

fn keep(dir: &Path, bytes: &[u8], days_ago: i64) {
    let now = chrono::Utc::now().timestamp();
    let meta = Meta { source: fetch::MANUAL_URL.into(), modified: Some("Wed, 22 Apr 2026 00:08:20 GMT".into()), fetched: now - days_ago * 86_400, checked: now - 2 * 3600, sample: true, ..Meta::default() };
    fetch::keep(dir, bytes, meta, &fetch::Progress::default()).expect("the sample kept");
}

fn book(dir: &Path) -> Rc<Book> {
    Rc::new(Book::open(dir).expect("the sample's book"))
}

/// Where an entry is in a book, by its ID.
fn at(book: &Book, id: &str) -> usize {
    book.items.iter().position(|i| i.id == id).unwrap_or_else(|| panic!("{id} in the sample"))
}

pub fn gallery(dir: &Path) {
    std::fs::create_dir_all(dir).expect("create the shots folder");
    let tones = [false, true];
    let sizes = [TextSize::Compact, TextSize::Standard, TextSize::Large];
    let mut shots: Vec<(String, usize, usize, Vec<u32>)> = Vec::new();
    let state = std::env::temp_dir().join(format!("catalyst-rulebook-shots-{}", std::process::id()));
    for (ti, light) in tones.iter().enumerate() {
        for (si, size) in sizes.iter().enumerate() {
            std::fs::remove_dir_all(&state).ok();
            std::fs::create_dir_all(&state).expect("a state folder");
            std::env::set_var("CATALYST_CONF", state.join("robot"));
            let books = state.join("rulebook");
            std::env::set_var("CATALYST_RULEBOOK", &books);
            let mut shell = crate::fullheight::shell("bezel", *size);
            shell.settings.screen_off = 0;
            shell.settings.light = *light;
            shell.apply_settings();
            settle(&mut shell, 60);
            let out = dir.join(format!("{}-{}", if *light { "light" } else { "dark" }, size.name()));
            std::fs::create_dir_all(&out).expect("a folder for the look");
            let mut save = |shell: &mut Shell, name: &str| {
                shell.draw();
                crate::platform::sim::save_png(&shell.gfx.canvas, &out.join(format!("{name}.png")), 2);
                shots.push((name.to_owned(), ti, si, shell.gfx.canvas.px.clone()));
            };
            use Key::*;
            // No manual yet, and getting it.
            shell.show(Box::new(Rulebook::waiting()));
            settle(&mut shell, 90);
            save(&mut shell, "01-get-the-manual");
            press(&mut shell, &[Back]);
            shell.show(Box::new(Rulebook::downloading(0.42)));
            settle(&mut shell, 90);
            save(&mut shell, "02-downloading");
            press(&mut shell, &[Back]);
            // Kept: the contents, a chapter, a rule.
            keep(&books, &sample::bytes(), 3);
            super::toggle_bookmark("G101");
            super::toggle_bookmark("R202");
            shell.show(Box::new(Rulebook::waiting()));
            settle(&mut shell, 90);
            save(&mut shell, "03-contents");
            let b = book(&books);
            let chapter = b.chapters().iter().position(|&c| b.items[c].id == "7").expect("chapter 7");
            // Search, Ask, Bookmarks, then the chapters.
            press(&mut shell, &vec![Down; 3 + chapter]);
            press(&mut shell, &[Enter]);
            save(&mut shell, "04-chapter");
            press(&mut shell, &[Down, Enter]);
            save(&mut shell, "05-rule");
            press(&mut shell, &[Back, Back, Back]);
            let rules: Vec<usize> = (0..b.items.len()).filter(|&i| b.items[i].is_rule()).collect();
            let r202 = rules.iter().position(|&i| b.items[i].id == "R202").expect("R202");
            shell.show(Box::new(Reader::new(b.clone(), rules.clone(), r202, Vec::new())));
            settle(&mut shell, 90);
            save(&mut shell, "06-rule-with-a-list-and-a-blue-box");
            press(&mut shell, &[Back]);
            shell.show(Box::new(Reader::new(b.clone(), vec![at(&b, "6.2")], 0, Vec::new())));
            settle(&mut shell, 90);
            save(&mut shell, "07-section-with-a-table");
            press(&mut shell, &[Back]);
            // Search: words, marked; a rule's ID; a result opened, marked too.
            shell.show(Box::new(Searching::new(b.clone(), "bumper colour")));
            settle(&mut shell, 90);
            save(&mut shell, "08-search");
            press(&mut shell, &[Enter]);
            save(&mut shell, "09-search-result-opened");
            press(&mut shell, &[Back, Back]);
            shell.show(Box::new(Searching::new(b.clone(), "G30")));
            settle(&mut shell, 90);
            save(&mut shell, "10-search-by-rule-id");
            press(&mut shell, &[Back]);
            // Ask: before a question, answered by Claude, its rules, on this handheld.
            shell.show(Box::new(Asking::new(b.clone()).by(Who::Claude("demo".into()))));
            settle(&mut shell, 90);
            save(&mut shell, "11-ask");
            press(&mut shell, &[Back]);
            let question = "Can a student reach into the field to grab a game piece during a match?";
            shell.show(Box::new(Asking::new(b.clone()).by(Who::Claude("demo".into())).asking(question)));
            pause(&mut shell, 500);
            save(&mut shell, "12-ask-answered-by-claude");
            press(&mut shell, &[Down, Down]);
            save(&mut shell, "13-ask-the-rules-cited");
            press(&mut shell, &[Back]);
            shell.show(Box::new(Asking::new(b.clone()).by(Who::Local).asking("how much can the robot weigh")));
            pause(&mut shell, 500);
            save(&mut shell, "14-ask-on-this-handheld");
            press(&mut shell, &[Back]);
            // An update: what it changed.
            keep(&books, &sample::revised(), 0);
            let b = book(&books);
            shell.show(Box::new(Rulebook::waiting()));
            settle(&mut shell, 90);
            press(&mut shell, &[End]);
            save(&mut shell, "15-contents-after-an-update");
            shell.show(Box::new(ManualPage::new(b.clone())));
            settle(&mut shell, 90);
            save(&mut shell, "16-manual");
            press(&mut shell, &[Down, Enter]);
            save(&mut shell, "17-what-changed");
            press(&mut shell, &[Back, Back]);
            shell.show(Box::new(Entries::bookmarks(b.clone())));
            settle(&mut shell, 90);
            save(&mut shell, "18-bookmarks");
            println!("18 pages in {}", out.display());
        }
    }
    std::fs::remove_dir_all(&state).ok();
    store::remove(&state);
    sheets(dir, &shots, tones.len(), sizes.len());
}

/// Every page's six shots in one sheet.
fn sheets(dir: &Path, shots: &[(String, usize, usize, Vec<u32>)], rows: usize, cols: usize) {
    let (w, h, scale, gap) = (320usize, 170usize, 2usize, 12usize);
    let mut names: Vec<&str> = shots.iter().map(|s| s.0.as_str()).collect();
    names.sort();
    names.dedup();
    for name in names {
        let sw = cols * w * scale + (cols + 1) * gap;
        let sh = rows * h * scale + (rows + 1) * gap;
        let mut rgb = vec![0x40u8; sw * sh * 3];
        for (_, row, col, px) in shots.iter().filter(|s| s.0 == name) {
            let (ox, oy) = (gap + col * (w * scale + gap), gap + row * (h * scale + gap));
            for y in 0..h * scale {
                for x in 0..w * scale {
                    let p = px[(y / scale) * w + x / scale];
                    let at = ((oy + y) * sw + ox + x) * 3;
                    rgb[at..at + 3].copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
                }
            }
        }
        let file = std::io::BufWriter::new(std::fs::File::create(dir.join(format!("sheet-{name}.png"))).expect("a sheet"));
        let mut enc = png::Encoder::new(file, sw as u32, sh as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().and_then(|mut wr| wr.write_image_data(&rgb)).expect("a sheet written");
    }
}
