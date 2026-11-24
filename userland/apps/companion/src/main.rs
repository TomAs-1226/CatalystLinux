//! Catalyst Companion: the handheld, reached from a Windows computer.
//!
//! Found by itself on the network or over the USB cable, and paired once
//! with the code Settings › Computer companion shows. Then: YouTube Music
//! and GitHub signed in with a click each, songs dropped into its library,
//! match logs, photos and notes brought over, its settings changed, its
//! system updated, and /data backed up to a zip and restored.
//!
//!     catalyst-companion                     the window
//!     catalyst-companion --shot DIR          its pages, as PNGs (made-up handheld)
//!     catalyst-companion --check HOST --code 123456
//!                                            pairs and tries every request, no window
//!     catalyst-companion --youtube-sign-in   the sign-in window (the app starts it)
//!     catalyst-companion --claude-bridge     Claude on the handheld: the bridge, in the tray
//!     catalyst-companion --claude-approver   its permission tool (Claude Code runs it)
//!     catalyst-companion --claude-shot DIR   the bridge's window, as PNGs (made-up computer)
//!
//! Trying the window where no one sits at it: `CATALYST_COMPANION_SHOTS=DIR`
//! saves the window on F12 (and on a driven `shot`), and
//! `CATALYST_COMPANION_DRIVE=FILE` plays its lines as input, one a line:
//! `wait S`, `key enter|up|down|tab|escape|backspace`, `type TEXT`,
//! `code-from FILE`, `click X Y`, `drop PATH`, `shot NAME`, `quit`.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod backup;
mod claude;
mod github;
mod jobs;
mod net;
mod store;
#[cfg(windows)]
mod tray;
mod ui;
mod win;
mod youtube;

use std::path::PathBuf;

use bezel::{Canvas, Icons, Text, Tokens};
use serde_json::json;

use crate::ui::{Ui, H, K, W};

fn tokens() -> Tokens {
    Tokens::of(bezel::Accent::Rose, bezel::Tone::Dark)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "--youtube-sign-in") {
        youtube::window();
    }
    // Claude on the handheld (claude.rs): the bridge, and the permission
    // tool its turns of Claude Code run.
    if args.iter().any(|a| a == "--claude-approver") {
        std::process::exit(claude::approver_main());
    }
    if args.iter().any(|a| a == "--claude-bridge") {
        std::process::exit(claude::bridge_main(args.iter().any(|a| a == "--show")));
    }
    if let Some(dir) = value("--claude-shot") {
        return claude::shots(PathBuf::from(dir));
    }
    if let Some(dir) = value("--shot") {
        return shots(PathBuf::from(dir));
    }
    if let Some(host) = value("--check") {
        let (host, port) = host.split_once(':').map_or((host.clone(), catalyst_companion_proto::PORT), |(h, p)| (h.to_owned(), p.parse().unwrap_or(catalyst_companion_proto::PORT)));
        std::process::exit(check(&host, port, &value("--code").unwrap_or_default()));
    }
    window();
}

/// One frame's worth of input, gathered from the window.
#[cfg(not(target_arch = "aarch64"))]
struct Typed(std::rc::Rc<std::cell::RefCell<Vec<char>>>);

#[cfg(not(target_arch = "aarch64"))]
impl minifb::InputCallback for Typed {
    fn add_char(&mut self, c: u32) {
        if let Some(c) = char::from_u32(c) {
            self.0.borrow_mut().push(c);
        }
    }
}

#[cfg(not(target_arch = "aarch64"))]
fn window() {
    use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
    let mut win = Window::new("Catalyst Companion", W, H, WindowOptions::default()).expect("a window");
    win.set_target_fps(60);
    let typed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    win.set_input_callback(Box::new(Typed(typed.clone())));
    win::accept_drops(win.get_window_handle());
    let mut app = app::App::new();
    app.window = win.get_window_handle() as usize;
    let (mut canvas, mut text, mut icons) = (Canvas::new(W as i32, H as i32), Text::new(), Icons::new());
    let mut focus: Option<String> = None;
    let mut was_down = false;
    let started = std::time::Instant::now();
    let shots = std::env::var_os("CATALYST_COMPANION_SHOTS").map(PathBuf::from);
    let mut drive = std::env::var_os("CATALYST_COMPANION_DRIVE").and_then(|f| std::fs::read_to_string(f).ok()).map(|t| t.lines().map(str::to_owned).collect::<std::collections::VecDeque<_>>());
    let mut drive_at = std::time::Instant::now();
    let mut fields: Vec<String> = Vec::new();
    while win.is_open() {
        let ctrl = win.is_key_down(Key::LeftCtrl) || win.is_key_down(Key::RightCtrl);
        let shift = win.is_key_down(Key::LeftShift) || win.is_key_down(Key::RightShift);
        let mut keys = Vec::new();
        let mut shot = false;
        for key in win.get_keys_pressed(KeyRepeat::Yes) {
            keys.push(match key {
                Key::Tab if shift => K::ShiftTab,
                Key::Tab => K::Tab,
                Key::Enter | Key::NumPadEnter => K::Enter,
                Key::Backspace => K::Backspace,
                Key::Delete => K::Delete,
                Key::Escape => K::Escape,
                Key::V if ctrl => K::Paste,
                Key::Up => K::Up,
                Key::Down => K::Down,
                Key::Left => K::Left,
                Key::Right => K::Right,
                Key::Home => K::Home,
                Key::End => K::End,
                Key::F12 => {
                    shot = true;
                    continue;
                }
                _ => continue,
            });
        }
        // Tab and Shift-Tab go through the fields; Escape leaves them.
        if keys.contains(&K::Tab) || keys.contains(&K::ShiftTab) {
            if !fields.is_empty() {
                let at = focus.as_ref().and_then(|f| fields.iter().position(|x| x == f));
                let n = fields.len();
                let next = match (at, keys.contains(&K::ShiftTab)) {
                    (Some(i), false) => (i + 1) % n,
                    (Some(i), true) => (i + n - 1) % n,
                    (None, _) => 0,
                };
                focus = Some(fields[next].clone());
            }
        }
        if keys.contains(&K::Escape) && focus.is_some() {
            focus = None;
            keys.retain(|k| *k != K::Escape);
        }
        let down = win.get_mouse_down(MouseButton::Left);
        let mut clicked = down && !was_down;
        was_down = down;
        let mut mouse = win.get_mouse_pos(MouseMode::Pass).unwrap_or((-1.0, -1.0));
        let wheel = win.get_scroll_wheel().map_or(0.0, |(_, y)| y.signum() * y.abs().min(3.0));
        let mut dropped = win::take_dropped();
        let mut chars: Vec<char> = typed.borrow_mut().drain(..).filter(|c| !c.is_control()).collect();
        let mut shot_name = None;
        // A run driven from a file, for trying the window with no one at it.
        if let Some(lines) = drive.as_mut().filter(|_| std::time::Instant::now() >= drive_at) {
            drive_at = std::time::Instant::now() + std::time::Duration::from_millis(150);
            match lines.pop_front().as_deref().map(|l| l.split_once(' ').unwrap_or((l, ""))) {
                Some(("wait", s)) => drive_at = std::time::Instant::now() + std::time::Duration::from_secs_f32(s.trim().parse().unwrap_or(1.0)),
                Some(("key", k)) => keys.extend(match k.trim() {
                    "enter" => Some(K::Enter),
                    "up" => Some(K::Up),
                    "down" => Some(K::Down),
                    "tab" => Some(K::Tab),
                    "escape" => Some(K::Escape),
                    "backspace" => Some(K::Backspace),
                    _ => None,
                }),
                Some(("type", text)) => chars.extend(text.chars()),
                Some(("code-from", file)) => chars.extend(std::fs::read_to_string(file.trim()).unwrap_or_default().lines().next().unwrap_or_default().chars()),
                Some(("click", at)) => {
                    let mut xy = at.split_whitespace().filter_map(|v| v.parse::<f32>().ok());
                    if let (Some(x), Some(y)) = (xy.next(), xy.next()) {
                        (mouse, clicked) = ((x, y), true);
                    }
                }
                Some(("drop", path)) => dropped.push(PathBuf::from(path.trim())),
                Some(("shot", name)) => shot_name = Some(name.trim().to_owned()),
                Some(("quit", _)) => return,
                _ => {}
            }
        }
        if !dropped.is_empty() {
            app.dropped(dropped);
        }
        let pasted = keys.contains(&K::Paste).then(win::clipboard_text).flatten();
        if clicked {
            // A click anywhere but a field lets go of the keys; the field clicked takes them.
            focus = None;
        }
        app.poll();
        let mut ui = Ui { canvas: &mut canvas, text: &mut text, icons: &mut icons, t: tokens(), mouse, clicked, wheel, typed: chars, keys, focus: &mut focus, fields: Vec::new(), pointing: false, time: started.elapsed().as_secs_f32(), pasted };
        app.frame(&mut ui);
        fields = std::mem::take(&mut ui.fields);
        if shot || shot_name.is_some() {
            if let Some(dir) = &shots {
                std::fs::create_dir_all(dir).ok();
                let name = shot_name.unwrap_or_else(|| format!("live-{}", started.elapsed().as_millis()));
                save_png(&canvas, &dir.join(format!("{name}.png")));
            }
        }
        win.update_with_buffer(&canvas.px, W, H).ok();
    }
}

#[cfg(target_arch = "aarch64")]
fn window() {}

fn save_png(canvas: &Canvas, path: &std::path::Path) {
    save_png_size(canvas, path, W, H);
}

/// A canvas `w` × `h` as a PNG.
pub(crate) fn save_png_size(canvas: &Canvas, path: &std::path::Path, w: usize, h: usize) {
    let file = std::fs::File::create(path).expect("a png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("png");
    let rgb: Vec<u8> = canvas.px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    w.write_image_data(&rgb).expect("png");
}

/// Its pages as PNGs, with a made-up handheld.
fn shots(dir: PathBuf) {
    use app::{Entry, Phase, Sent, Upload};
    std::fs::create_dir_all(&dir).expect("the folder");
    let now = json!({"name": "catalyst", "team": "5805", "accent": "rose", "mode": "dark", "text": "standard", "home": "grid", "open_robot": true, "match_report": true, "ssh": false, "calm": false,
        "keys": {"tba": "set · …9f2c", "nexus": "not set", "openai": "not set", "claude": "set · …a41b", "github": "set · …7Qx2"}, "music": true, "github": "tomas-1226"});
    let update = json!({"version": "0.7.0", "available": true, "status": {"state": "available", "version": "0.7.1", "size": 48_200_000, "progress": 0.0}});
    let (mut canvas, mut text, mut icons) = (Canvas::new(W as i32, H as i32), Text::new(), Icons::new());
    let mut focus: Option<String> = None;
    let mut draw = |app: &mut app::App, name: &str, focus: &mut Option<String>| {
        // Enough frames for the springs to come to rest.
        for _ in 0..90 {
            let mut ui = Ui { canvas: &mut canvas, text: &mut text, icons: &mut icons, t: tokens(), mouse: (-1.0, -1.0), clicked: false, wheel: 0.0, typed: Vec::new(), keys: Vec::new(), focus, fields: Vec::new(), pointing: false, time: 0.2, pasted: None };
            app.frame(&mut ui);
        }
        save_png(&canvas, &dir.join(format!("{name}.png")));
    };
    let song = |n: &str, s: u64| Entry { name: n.into(), dir: false, size: s, modified: 1_763_800_000 };
    let folder = |n: &str| Entry { name: n.into(), dir: true, size: 0, modified: 1_763_700_000 };

    let mut app = app::App::for_shots(now.clone(), update.clone());
    app.phase = Phase::Find;
    app.found = vec![
        net::Found { id: "a".into(), name: "catalyst".into(), version: "0.7.0".into(), host: "192.168.1.42".into(), port: 47315, usb: false },
        net::Found { id: "b".into(), name: "pit-handheld".into(), version: "0.7.0".into(), host: "192.168.7.1".into(), port: 47315, usb: true },
    ];
    draw(&mut app, "1-find", &mut focus);
    app.phase = Phase::Pair(app.found[0].clone());
    app.code = "4829".into();
    draw(&mut app, "2-pair", &mut focus);
    app.code = "482913".into();
    app.pair_error = Some("that code did not match: check it on the handheld and type it again".into());
    draw(&mut app, "2b-pair-wrong", &mut focus);

    let mut app = app::App::for_shots(now.clone(), update.clone());
    draw(&mut app, "3-sign-ins", &mut focus);
    app.gh = app::Gh::Consent;
    draw(&mut app, "3b-github-consent", &mut focus);
    app.gh = app::Gh::Idle;
    app.toast = Some(("catalyst is signed in to YouTube Music".into(), false, std::time::Instant::now()));
    draw(&mut app, "3c-signed-in", &mut focus);
    app.toast = None;

    app.page = app::Page::Music;
    app.lib.entries = Some(Ok(vec![folder("Daft Punk"), folder("From computer"), folder("Hans Zimmer"), folder("Playlists"), song("Interstellar Main Theme.flac", 41_300_000), song("Robot Rock.mp3", 9_200_000)]));
    {
        let mut q = app.uploads.lock().unwrap();
        for (n, s, state) in [("Harder, Better, Faster, Stronger.mp3", 8_100_000, Sent::Done("From computer/x".into())), ("Time.flac", 58_000_000, Sent::Sending), ("Cornfield Chase.m4a", 6_400_000, Sent::Waiting)] {
            let u = Upload { path: PathBuf::from(n), size: s, state, progress: Default::default() };
            u.progress.set(0.62);
            q.push(u);
        }
    }
    draw(&mut app, "4-music", &mut focus);
    app.uploads.lock().unwrap().clear();

    app.page = app::Page::Files;
    app.files.entries = Some(Ok(vec![folder("2026 Houston"), song("Q31 · 2026-04-18 14-02.wpilog", 3_400_000), song("Q32 · 2026-04-18 14-31.wpilog", 3_100_000), song("Practice 2 · 2026-04-17.wpilog", 12_800_000)]));
    draw(&mut app, "5-files", &mut focus);

    app.page = app::Page::Settings;
    app.form.ssid = "Pit Wi-Fi".into();
    app.form.psk = "hunter22".into();
    app.form.light = true;
    focus = Some("psk".into());
    draw(&mut app, "6-settings", &mut focus);
    focus = None;
    app.form.ssid.clear();
    app.form.psk.clear();
    app.form.light = false;

    app.page = app::Page::System;
    app.last_backup = Some(PathBuf::from("catalyst-backup-2026-11-22.zip"));
    draw(&mut app, "7-system", &mut focus);
    app.update = Some(json!({"version": "0.7.0", "available": true, "status": {"state": "installing", "version": "0.7.1", "progress": 0.64}}));
    app.restore = Some((PathBuf::from("catalyst-backup-2026-11-20.zip"), 1204, 0));
    draw(&mut app, "7b-system-installing-restore", &mut focus);
    println!("companion pages in {}", dir.display());
}

/// `--check HOST --code CODE`: pairs with the handheld at HOST and makes
/// every kind of request once, as the window would, saying how each went
/// (never what was sent). Unpairs at the end. Exit 0 if all went.
fn check(host: &str, port: u16, code: &str) -> i32 {
    use std::io::Read;
    let mut failed = 0;
    let mut step = |name: &str, r: Result<String, String>| match r {
        Ok(note) => println!("ok      {name}{}", if note.is_empty() { String::new() } else { format!(": {note}") }),
        Err(why) => {
            failed += 1;
            println!("FAILED  {name}: {why}");
        }
    };
    let found = net::discover(std::time::Duration::from_millis(1200));
    step("discovery", Ok(found.iter().map(|f| format!("{} at {} ({})", f.name, f.host, if f.usb { "usb" } else { "network" })).collect::<Vec<_>>().join(", ")));
    let paired = match net::pair(host, port, code, &win::computer_name()) {
        Ok(p) => p,
        Err(why) => {
            println!("FAILED  pairing: {why}");
            return 1;
        }
    };
    step("pairing", Ok(format!("{} {} · certificate pinned", paired.name, paired.version)));
    let t = paired.target.clone();
    let mut wrong = t.clone();
    wrong.token = "0".repeat(64);
    step("a wrong token is refused", match wrong.get("/v1/status") {
        Err(why) if why.contains("not paired") => Ok(String::new()),
        other => Err(format!("{other:?}")),
    });
    let mut other_cert = t.clone();
    other_cert.pin = [7; 32];
    step("another certificate is refused", match other_cert.get("/v1/status") {
        Err(why) if why.contains("another certificate") => Ok(String::new()),
        other => Err(format!("{other:?}")),
    });
    step("status", t.get("/v1/status").map(|v| format!("free {:?}", v["free"].as_u64())));
    step("settings read", t.get("/v1/settings").map(|v| format!("accent {} · music signed in {}", v["accent"], v["music"])));
    step("settings sent", t.post("/v1/settings", json!({"accent": "blue", "mode": "light"})).map(|v| v["received"].to_string()));
    let not_signed_in = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t1900000000\tPREF\tf6=40000000\n";
    step("cookies that would not sign in are refused", match t.call("POST", "/v1/music/sign-in", net::Send::Text(not_signed_in.into()), net::Sink::Memory, std::time::Duration::from_secs(20)).and_then(|a| a.ok()) {
        Err(why) => Ok(why),
        Ok(v) => Err(format!("taken: {v}")),
    });
    let jar = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tcheck-sapisid/AbCd\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1900000000\t__Secure-3PSID\tcheck-session\n";
    step("youtube music sign-in", t.call("POST", "/v1/music/sign-in", net::Send::Text(jar.into()), net::Sink::Memory, std::time::Duration::from_secs(20)).and_then(|a| a.ok()).map(|v| v["received"].to_string()));
    step("github token", t.post("/v1/github", json!({"token": "ghp_companioncheck0000000000000000000000"})).map(|_| String::new()));
    step("github signed out", t.delete("/v1/github").map(|_| String::new()));
    // A song there and back.
    let song: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let mut from = &song[..];
    let mut sent = 0u64;
    let mut tell = |n: u64| sent = n;
    let up = t.call("PUT", "/v1/music?name=Companion%20Check.mp3", net::Send::Stream { len: song.len() as u64, from: &mut from, progress: &mut tell }, net::Sink::Memory, std::time::Duration::from_secs(60)).and_then(|a| a.ok());
    let saved = up.as_ref().ok().and_then(|v| v["saved"].as_str().map(str::to_owned)).unwrap_or_default();
    step("song added", up.map(|_| format!("{saved} ({} KB)", sent >> 10)));
    step("library listed", t.get("/v1/files?area=music&path=From%20computer").map(|v| v["entries"].as_array().map_or(0, Vec::len).to_string() + " in From computer"));
    let mut back = Vec::new();
    let mut got = |_: u64, _: Option<u64>| {};
    let down = t.call("GET", &format!("/v1/file?area=music&path={}", net::encode(&saved)), net::Send::None, net::Sink::Writer { to: &mut back, progress: &mut got }, std::time::Duration::from_secs(60));
    step("song downloaded whole", down.and_then(|a| if a.status == 200 && back == song { Ok(format!("{} KB, the same bytes", back.len() >> 10)) } else { Err(format!("status {} · {} bytes", a.status, back.len())) }));
    // A backup to a zip, and back.
    let dir = std::env::temp_dir().join(format!("companion-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok();
    let zip = dir.join("backup.zip");
    let (mut writer, reader) = jobs::pipe();
    let zip_to = zip.clone();
    let zipping = std::thread::spawn(move || backup::tar_to_zip(reader, &zip_to));
    let mut tell = |_: u64, _: Option<u64>| {};
    let fetched = t.call("GET", "/v1/backup", net::Send::None, net::Sink::Writer { to: &mut writer, progress: &mut tell }, std::time::Duration::from_secs(60));
    drop(writer);
    let zipped = zipping.join().unwrap_or_else(|_| Err("the zip thread stopped".into()));
    step("backup to a zip", fetched.and_then(|_| zipped).map(|n| format!("{n} files, {} KB", zip.metadata().map_or(0, |m| m.len()) >> 10)));
    step("restore from the zip", backup::Restore::open(&zip).and_then(|r| {
        let len = r.len;
        let mut stream = backup::Stream::new(r);
        let mut tell = |_: u64| {};
        t.call("POST", "/v1/restore", net::Send::Stream { len, from: &mut stream, progress: &mut tell }, net::Sink::Memory, std::time::Duration::from_secs(120))?.ok().map(|v| format!("{} files", v["restored"]))
    }));
    step("update status", t.get("/v1/update").map(|v| format!("updater here: {}", v["available"])));
    step("unpaired", t.delete("/v1/pair").map(|_| String::new()));
    step("the token is no good after", match t.get("/v1/status") {
        Err(why) if why.contains("not paired") => Ok(String::new()),
        other => Err(format!("{other:?}")),
    });
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).ok();
    std::fs::remove_dir_all(&dir).ok();
    i32::from(failed > 0)
}
