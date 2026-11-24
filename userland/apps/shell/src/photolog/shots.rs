//! Photo documentation's pictures (`catalyst-shell --photolog-shots DIR`):
//! a caption under its photo (and the chip that says it is there), a photo
//! waiting for its caption, Settings › Photos (on, and as it starts: off),
//! naming a place (typed, and picked), the named places, the build log (and
//! a day about to go to Team), its export, and a caption being written; in
//! dark and light at each text size, with a sheet of each (dark above
//! light; compact, standard, large). The export's Markdown is written
//! beside them.

use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime};

use super::meta::About;
use super::places;
use super::store::{Job, Record};
use super::ui::{BuildLog, CaptionPage, ExportPage, NamePlace, PhotoSettings, PlacesPage};
use crate::gfx::TextSize;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Screen, Shell};

/// The album: (stamp, scene time, caption, place, event context).
const ALBUM: [(&str, f32, &str, &str, &str); 9] = [
    ("20261121_154210", 3.0, "Bumpers back on after the bolt swap, the robot carried out of the pit toward queueing.", "Pit", "before Q32"),
    ("20261121_143005", 9.0, "The robot on the field during the match, intake down and driving toward the far alliance wall.", "", "during Q31"),
    ("20261121_102214", 21.0, "Pit crew replacing two bumper bolts on the left side after the morning's practice match.", "Pit", ""),
    ("20261121_091530", 30.0, "", "Pit", ""),
    ("20261119_184500", 36.0, "Drive base on the shop floor with the new swerve modules fitted, wiring still loose at the back.", "Shop", ""),
    ("20261119_172201", 40.0, "Testing the elevator's top stop: the carriage at full height with the belt tensioned.", "Shop", ""),
    ("20261119_160130", 44.0, "The intake rollers off the robot on the bench, compliant wheels laid out beside them.", "Shop", ""),
    ("20261117_101532", 50.0, "The robot on the practice field at the end of the afternoon, bumpers on, lined up for a drive test.", "", ""),
    ("20261117_100410", 52.0, "", "", ""),
];

fn when(stamp: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(stamp, "%Y%m%d_%H%M%S").expect("a stamp")
}

fn press(screen: &mut dyn Screen, key: Key) {
    let mut robot = Robot::none();
    let mut settings = crate::shell::Settings::default();
    let mut say = None;
    screen.key(key, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
}

/// The album, its sidecars and records, the places, the month's spend.
fn stage() {
    let photos = crate::photos::dir();
    std::fs::create_dir_all(&photos).expect("the album");
    std::fs::write(photos.join(".seeded"), b"").ok();
    std::fs::create_dir_all(super::dir()).ok();
    std::fs::write(super::dir().join(".seeded"), b"").ok();
    super::reload();
    let shop = places::demo_place("shop");
    let pit = places::demo_place("pit");
    super::with(|l| {
        l.prefs.captions = true;
        l.prefs.town = true;
        l.places.name("Shop", &shop);
        l.places.name("Pit", &pit);
        l.places.name("Home", &places::demo_place("home"));
        for (k, (stamp, t, caption, place, context)) in ALBUM.iter().enumerate() {
            let name = format!("IMG_{stamp}.jpg");
            let path = photos.join(&name);
            crate::camera::save_jpeg(&path, &crate::camera::scene(800, 600, *t, false)).expect("a photo");
            let at = super::unix(when(stamp));
            let fp = match *place {
                "Shop" => shop.clone(),
                "Pit" => pit.clone(),
                _ => Vec::new(),
            };
            let event = stamp.starts_with("20261121");
            let about = About {
                caption: caption.to_string(),
                by: if caption.is_empty() { String::new() } else if k == 2 { "you".into() } else { super::caption::MODEL.into() },
                place: place.to_string(),
                event: if event { "Finger Lakes Regional".into() } else { String::new() },
                venue: if event { "Rochester, NY".into() } else { String::new() },
                context: context.to_string(),
                town: if stamp.starts_with("20261117") { "Henrietta, NY".into() } else { String::new() },
            };
            super::meta::write(&path, &about).ok();
            let caption_job = match (caption.is_empty(), k) {
                (true, 3) => Job::Waiting { tries: 0, next: at },
                (true, _) => Job::None,
                _ => Job::Done,
            };
            let cost = if caption.is_empty() || k == 2 { 0 } else { 3_900 + 120 * k as u64 };
            l.records.insert(name, Record { at, fp, caption: caption_job, town: Job::None, cost, spot: None });
        }
        let month = super::store::month_of(super::unix(when("20261121_120000")));
        l.spend.insert(month, l.records.values().map(|r| r.cost).sum());
        l.save();
    });
}

fn viewer(name: &str, open: bool) -> Box<dyn Screen> {
    let photos = crate::photos::list();
    let at = photos.iter().position(|p| p.name == name).unwrap_or(0);
    let mut v = crate::photos::Viewer::new(photos, at);
    if open {
        press(&mut v, Key::Down);
    }
    Box::new(v)
}

pub fn shots(dir: &Path) {
    std::fs::create_dir_all(dir).expect("the shots folder");
    let state = std::env::temp_dir().join(format!("catalyst-photolog-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&state).ok();
    std::fs::create_dir_all(&state).expect("a state folder");
    std::env::set_var("CATALYST_CONF", state.join("robot"));
    stage();
    let day = NaiveDate::from_ymd_opt(2026, 11, 21).expect("a day");

    type Page = Box<dyn Fn() -> Box<dyn Screen>>;
    let pages: Vec<(&str, Page)> = vec![
        ("photolog-1-caption", Box::new(|| viewer("IMG_20261121_143005.jpg", true))),
        ("photolog-2-caption-chip", Box::new(|| viewer("IMG_20261121_102214.jpg", false))),
        ("photolog-3-waiting", Box::new(|| viewer("IMG_20261121_091530.jpg", true))),
        (
            "photolog-4-settings",
            Box::new(|| {
                let mut s = PhotoSettings::new();
                s.has_key = true;
                Box::new(s)
            }),
        ),
        (
            "photolog-5-settings-off",
            Box::new(|| {
                super::with(|l| l.prefs = super::store::Prefs::default());
                let mut s = PhotoSettings::new();
                s.has_key = true;
                Box::new(s)
            }),
        ),
        (
            "photolog-6-name-place",
            Box::new(|| {
                let mut n = NamePlace::new("IMG_20261121_091530.jpg");
                n.field.text = "Practice field".into();
                Box::new(n)
            }),
        ),
        (
            "photolog-7-name-place-pick",
            Box::new(|| {
                let mut n = NamePlace::new("IMG_20261121_091530.jpg");
                n.in_list = true;
                n.list.focus = 1;
                Box::new(n)
            }),
        ),
        ("photolog-8-places", Box::new(|| Box::new(PlacesPage::new()))),
        ("photolog-9-log", Box::new(|| Box::new(BuildLog::new()))),
        ("photolog-9b-log-event", Box::new(|| Box::new(BuildLog::new().at("IMG_20261121_143005.jpg")))),
        (
            "photolog-10-log-to-team",
            Box::new(move || {
                let mut log = BuildLog::new().at("IMG_20261119_172201.jpg");
                log.sending = Some(NaiveDate::from_ymd_opt(2026, 11, 19).expect("a day"));
                Box::new(log)
            }),
        ),
        (
            "photolog-11-export",
            Box::new(move || {
                let photos = crate::photos::list();
                let mut page = ExportPage::new(super::log::gather(&photos), day);
                page.export_now(super::log::Range::Event, true);
                Box::new(page)
            }),
        ),
        (
            "photolog-12-caption-edit",
            Box::new(|| {
                let path = crate::photos::dir().join("IMG_20261121_102214.jpg");
                Box::new(CaptionPage::new(&path, "Pit crew replacing two bumper bolts on the left side after the morning's practice match. Spare bolts are in the red bin"))
            }),
        ),
    ];
    let mut sheets: Vec<Vec<bezel::Canvas>> = (0..pages.len()).map(|_| Vec::new()).collect();
    for light in [false, true] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            for (n, (name, make)) in pages.iter().enumerate() {
                super::with(|l| {
                    l.prefs.captions = true;
                    l.prefs.town = true;
                });
                let mut shell: Shell = crate::fullheight::shell("catalyst", size);
                shell.settings.light = light;
                shell.settings.clock24 = true;
                shell.apply_settings();
                crate::fullheight::settle(&mut shell, 30);
                shell.show(make());
                // Thumbnails and decodes come from their threads. The chip
                // shows with the viewer's chrome, which fades after 2.5 s.
                if *name == "photolog-2-caption-chip" {
                    crate::fullheight::settle(&mut shell, 40);
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    crate::fullheight::settle(&mut shell, 4);
                } else {
                    for _ in 0..6 {
                        crate::fullheight::settle(&mut shell, 20);
                        std::thread::sleep(std::time::Duration::from_millis(40));
                    }
                    crate::fullheight::settle(&mut shell, 60);
                }
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
    // The export as written: its Markdown beside the pictures.
    let exported = super::log::exports_dir();
    if let Some(folder) = std::fs::read_dir(&exported).ok().and_then(|mut d| d.find_map(|e| e.ok().map(|e| e.path()))) {
        std::fs::copy(folder.join("build-log.md"), dir.join("photolog-export-build-log.md")).ok();
    }
    std::fs::remove_dir_all(&state).ok();
    println!("photo documentation's pages in {}", dir.display());
}
