//! The phone gallery's pictures (`catalyst-shell --gallery-shots DIR`): the
//! sheet on the handheld as it shares (waiting, then two phones with
//! photos on their way, and its keys), with no network, and stopped; the
//! album's hint and Share's row that lead to it; in dark and light at each
//! text size, with a sheet of each (dark above light; compact, standard,
//! large). Then the page a phone is given, as files to open in a browser
//! (`page/`): the album, some chosen, a photo open, a download on its way,
//! the way in by the code, and the page once sharing has stopped.
//!
//! The server listens on this computer's loopback only, while the sheet
//! shows a phone's hotspot address (`192.168.43.27`).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

use super::page::{self, Look};
use super::sheet::Showing;
use super::{Gallery, GalleryScreen, Stopped};
use crate::gfx::TextSize;
use crate::photolog::meta::About;
use crate::platform::Key;
use crate::shell::{Screen, Shell};

/// The address a phone's hotspot gives (Android's), shown on the sheet.
const HOTSPOT_ADDRESS: &str = "192.168.43.27";

/// (stamp, scene time, caption, named place, event, match)
const ALBUM: [(&str, f32, &str, &str, &str, &str); 9] = [
    ("20261121_154210", 3.0, "Bumpers back on after the bolt swap, the robot carried out of the pit toward queueing.", "Pit", "Finger Lakes Regional", "before Q32"),
    ("20261121_143005", 9.0, "The robot on the field during the match, intake down and driving toward the far alliance wall.", "", "Finger Lakes Regional", "during Q31"),
    ("20261121_102214", 21.0, "Pit crew replacing two bumper bolts on the left side after the morning's practice match.", "Pit", "Finger Lakes Regional", ""),
    ("20261121_091530", 30.0, "", "Pit", "Finger Lakes Regional", ""),
    ("20261119_184500", 36.0, "Drive base on the shop floor with the new swerve modules fitted, wiring still loose at the back.", "Shop", "", ""),
    ("20261119_172201", 40.0, "Testing the elevator's top stop: the carriage at full height with the belt tensioned.", "Shop", "", ""),
    ("20261119_160130", 44.0, "The intake rollers off the robot on the bench, compliant wheels laid out beside them.", "Shop", "", ""),
    ("20261117_101532", 50.0, "The robot on the practice field at the end of the afternoon, bumpers on, lined up for a drive test.", "", "", ""),
    ("20261117_100410", 52.0, "", "", "", ""),
];

/// The album: photos of the simulated camera's scene with what the build
/// log says of them, and a pixel loop.
fn stage(album: &Path) {
    std::fs::create_dir_all(album).expect("the album");
    std::fs::write(album.join(".seeded"), b"").ok();
    for (stamp, t, caption, place, event, context) in ALBUM {
        let path = album.join(format!("IMG_{stamp}.jpg"));
        crate::camera::save_jpeg(&path, &crate::camera::scene(1024, 768, t, false)).expect("a photo");
        let about = About {
            caption: caption.into(),
            by: if caption.is_empty() { String::new() } else { crate::photolog::caption::MODEL.into() },
            place: place.into(),
            event: event.into(),
            venue: if event.is_empty() { String::new() } else { "Rochester, NY".into() },
            context: context.into(),
            town: if stamp.starts_with("20261117") { "Henrietta, NY".into() } else { String::new() },
        };
        crate::photolog::meta::write(&path, &about).ok();
    }
    // A loop from pixel mode: the scene drifting, in whole pixels.
    let frames: Vec<crate::filters::pixel::Frame> = (0..8)
        .map(|i| {
            let pic = crate::camera::scene(80, 60, 12.0 + i as f32 * 0.4, false);
            let step = |v: u8| (u16::from(v) * 5 / 255) as u8;
            let colours: Vec<[u8; 3]> = (0..216u16).map(|c| [(c / 36 * 51) as u8, (c / 6 % 6 * 51) as u8, (c % 6 * 51) as u8]).collect();
            let idx = pic.rgb.chunks_exact(3).map(|p| step(p[0]) * 36 + step(p[1]) * 6 + step(p[2])).collect();
            crate::filters::pixel::Frame { w: 80, h: 60, idx, colours, palette: crate::filters::pixel::Palette::Tide }
        })
        .collect();
    std::fs::write(album.join("PIX_20261121_160500.gif"), crate::filters::gif::encode(&frames, 4, 12, "Pixel loop · tide")).ok();
    crate::photos::changed();
}

/// Sharing on this computer's loopback, the sheet saying the hotspot's address.
fn sharing(album: &Path, look: &Look) -> Gallery {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    listener.set_nonblocking(true).ok();
    Gallery::serve_on(listener, HOTSPOT_ADDRESS, 80, album.to_path_buf(), look.clone(), super::serve::OPEN_FOR).expect("sharing")
}

/// A phone scanning the code: in by the link, over the loopback.
fn phone_arrives(gallery: &Gallery, port: u16) {
    let path = gallery.link().split_once(HOTSPOT_ADDRESS).map(|(_, p)| p.to_owned()).unwrap_or_default();
    if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {HOTSPOT_ADDRESS}\r\n\r\n").as_bytes()).ok();
        let mut out = Vec::new();
        s.read_to_end(&mut out).ok();
    }
}

fn press(screen: &mut dyn Screen, key: Key) {
    let mut robot = crate::robot::Robot::none();
    let mut settings = crate::shell::Settings::default();
    let mut say = None;
    screen.key(key, &mut crate::shell::Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
}

pub fn shots(dir: &Path) {
    std::fs::create_dir_all(dir).expect("the shots folder");
    let state = std::env::temp_dir().join(format!("catalyst-gallery-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&state).ok();
    std::fs::create_dir_all(&state).expect("a state folder");
    std::env::set_var("CATALYST_CONF", state.join("robot"));
    std::fs::create_dir_all(crate::photolog::dir()).ok();
    std::fs::write(crate::photolog::dir().join(".seeded"), b"").ok();
    let album = crate::photos::dir();
    stage(&album);
    let look = Look { accent: crate::shell::Settings::default().accent, name: "catalyst-5805".into(), h24: true };

    type Page = Box<dyn Fn() -> Box<dyn Screen>>;
    let (a, l) = (album.clone(), look.clone());
    let sharing_page: Page = Box::new(move || Box::new(GalleryScreen::around(sharing(&a, &l), a.clone(), l.clone())));
    let (a, l) = (album.clone(), look.clone());
    let phones_page: Page = Box::new(move || {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a port");
        listener.set_nonblocking(true).ok();
        let port = listener.local_addr().expect("its port").port();
        let gallery = Gallery::serve_on(listener, HOTSPOT_ADDRESS, 80, a.clone(), l.clone(), super::serve::OPEN_FOR).expect("sharing");
        phone_arrives(&gallery, port);
        phone_arrives(&gallery, port);
        gallery.pretend(true, Duration::from_secs(581));
        Box::new(GalleryScreen::around(gallery, a.clone(), l.clone()))
    });
    let (a, l) = (album.clone(), look.clone());
    let none_page: Page = Box::new(move || Box::new(GalleryScreen::showing(a.clone(), l.clone(), Showing::NoNetwork)));
    let (a, l) = (album.clone(), look.clone());
    let stopped_page: Page = Box::new(move || Box::new(GalleryScreen::showing(a.clone(), l.clone(), Showing::Stopped(Stopped::TimeUp))));
    let album_page: Page = Box::new(|| {
        let mut album = crate::photos::Album::new();
        press(&mut album, Key::Right);
        Box::new(album)
    });
    let share_page: Page = Box::new(|| Box::new(crate::share::Share::new(&crate::robot::Robot::none())));
    // (name, the theme, the page, keys pressed once it shows)
    let pages: Vec<(&str, &str, Page, &[Key])> = vec![
        ("gallery-1-sharing", "catalyst", sharing_page, &[]),
        ("gallery-2-phones", "catalyst", phones_page, &[]),
        ("gallery-3-keys", "catalyst", Box::new({
            let (a, l) = (album.clone(), look.clone());
            move || Box::new(GalleryScreen::around(sharing(&a, &l), a.clone(), l.clone())) as Box<dyn Screen>
        }), &[Key::Char('?')]),
        ("gallery-4-no-network", "catalyst", none_page, &[]),
        ("gallery-5-stopped", "catalyst", stopped_page, &[]),
        ("gallery-6-bezel", "bezel", Box::new({
            let (a, l) = (album.clone(), look.clone());
            move || Box::new(GalleryScreen::around(sharing(&a, &l), a.clone(), l.clone())) as Box<dyn Screen>
        }), &[]),
        ("gallery-7-album-hint", "catalyst", album_page, &[]),
        ("gallery-8-share-row", "catalyst", share_page, &[]),
    ];
    let mut sheets: Vec<Vec<bezel::Canvas>> = (0..pages.len()).map(|_| Vec::new()).collect();
    for light in [false, true] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            for (n, (name, theme, make, keys)) in pages.iter().enumerate() {
                let mut shell: Shell = crate::fullheight::shell(theme, size);
                shell.settings.light = light;
                shell.settings.clock24 = true;
                shell.apply_settings();
                crate::fullheight::settle(&mut shell, 30);
                shell.show(make());
                for _ in 0..6 {
                    crate::fullheight::settle(&mut shell, 20);
                    std::thread::sleep(Duration::from_millis(30));
                }
                for key in keys.iter() {
                    shell.key(*key);
                }
                crate::fullheight::settle(&mut shell, 90);
                shell.draw();
                let file = format!("{name}-{}-{}.png", if light { "light" } else { "dark" }, size.name());
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(file), 2);
                sheets[n].push(shell.gfx.canvas.clone());
            }
        }
    }
    for ((name, ..), shots) in pages.iter().zip(&sheets) {
        crate::platform::sim::save_png(&crate::editor::shots::sheet(shots), &dir.join(format!("sheet-{name}.png")), 2);
    }
    write_page(&dir.join("page"), &album, &look);
    std::fs::remove_dir_all(&state).ok();
    println!("the phone gallery's sheet in {}, its page in {}", dir.display(), dir.join("page").display());
}

/// The page as files: each state as its own HTML, the thumbnails, photos
/// and fonts beside them where the page asks for them.
fn write_page(out: &Path, album: &Path, look: &Look) {
    use serde_json::json;
    for sub in ["t", "p", "d", "f"] {
        std::fs::create_dir_all(out.join(sub)).expect("the page's folders");
    }
    let items = super::items(album);
    for item in &items {
        if let Some(jpeg) = super::serve::thumb_jpeg(album, item) {
            std::fs::write(out.join("t").join(&item.photo.name), jpeg).ok();
        }
        std::fs::copy(&item.photo.path, out.join("p").join(&item.photo.name)).ok();
    }
    let (sans, code) = bezel::text::font_files();
    std::fs::write(out.join("f").join("sans.ttf"), sans).ok();
    std::fs::write(out.join("f").join("code.ttf"), code).ok();
    let chosen: Vec<&str> = items.iter().skip(1).take(4).map(|i| i.photo.name.as_str()).collect();
    let open = items.iter().position(|i| i.photo.name == "IMG_20261121_143005.jpg").unwrap_or(0);
    let open_quiet = items.iter().position(|i| i.photo.name == "IMG_20261117_100410.jpg").unwrap_or(0);
    let open_loop = items.iter().position(|i| i.moving()).unwrap_or(0);
    let total: u64 = items.iter().map(|i| i.photo.bytes).sum();
    let pages = [
        ("index.html", page::sample(look, &items, json!({}))),
        ("select.html", page::sample(look, &items, json!({ "select": chosen }))),
        ("viewer.html", page::sample(look, &items, json!({ "open": open }))),
        ("viewer-no-caption.html", page::sample(look, &items, json!({ "open": open_quiet }))),
        ("viewer-loop.html", page::sample(look, &items, json!({ "open": open_loop }))),
        ("download.html", page::sample(look, &items, json!({ "say": format!("{} photos on the way · {} MB", items.len(), (total as f64 / 1e6).round().max(1.0)), "icon": "download" }))),
        ("empty.html", page::sample(look, &[], json!({}))),
        ("orange.html", page::sample(&Look { accent: bezel::Accent::Orange, ..look.clone() }, &items, json!({}))),
        ("code.html", page::code_page(look, None)),
        ("code-wrong.html", page::code_page(look, Some("That isn't the code on the handheld's screen."))),
        ("closed.html", page::closed_page(look, Some(Stopped::TimeUp))),
    ];
    for (name, html) in pages {
        std::fs::write(out.join(name), html).ok();
    }
}
