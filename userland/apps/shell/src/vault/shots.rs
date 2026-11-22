//! `catalyst-shell --vault-shots <dir>`: the Vault app's pages, from demo
//! data (never a real secret), in both looks, dark and light, at each text
//! size: each page at twice the panel's size in
//! `<dir>/<look>-<dark|light>-<size>/`, and a sheet of all twelve as
//! `<dir>/sheet-<page>.png` (compact, standard, large across; down,
//! Catalyst dark and light, then Bezel dark and light).

use std::path::Path;
use std::time::{Duration, Instant};

use super::{Entry, Field, Kind};
use crate::fullheight::settle;
use crate::gfx::TextSize;
use crate::platform::Key;
use crate::shell::Shell;

const PASSCODE: &str = "2468";

fn login(title: &str, user: &str, password: &str, url: &str, totp: Option<&str>, notes: &str) -> Entry {
    let mut e = Entry::new(Kind::Login, super::entry::new_id(), 1_763_700_000);
    e.set(Field::Title, title);
    e.set(Field::Username, user);
    e.set(Field::Password, password);
    e.set(Field::Url, url);
    e.set(Field::Notes, notes);
    if let Some(t) = totp {
        e.set(Field::Totp, t);
    }
    e
}

/// The demo vault: logins, a Wi-Fi network, a card's PIN and a note.
fn demo() -> Vec<Entry> {
    let mut out = vec![
        login("GitHub", "pit-crew@example.org", "demo-Vk7#mQ2pXr9!", "https://github.com", Some("otpauth://totp/GitHub:pit-crew?secret=JBSWY3DPEHPK3PXP&issuer=GitHub"), "recovery codes are in the team binder"),
        login("The Blue Alliance", "mentor@example.org", "demo-tba-8841", "https://www.thebluealliance.com", None, ""),
        login("FRC Events", "team5805", "demo-frc-Qw3rty", "https://frc-events.example.org", Some("otpauth://totp/FIRST:team5805?secret=GEZDGNBVGY3TQOJQ&issuer=FIRST&digits=8"), ""),
        login("Scouting app", "scout", "demo-scout-2026", "https://scout.example.org", None, ""),
        login("Team email", "team5805@example.org", "demo-mail-77#", "https://mail.example.org", None, ""),
    ];
    let mut wifi = Entry::new(Kind::Wifi, super::entry::new_id(), 1_763_700_000);
    wifi.set(Field::Title, "Pit Wi-Fi");
    wifi.set(Field::Password, "demo-pit-network");
    wifi.set(Field::Notes, "5 GHz only at the venue");
    let mut pin = Entry::new(Kind::Pin, super::entry::new_id(), 1_763_700_000);
    pin.set(Field::Title, "Team debit card");
    pin.set(Field::Username, "4821");
    pin.set(Field::Password, "0000");
    let mut note = Entry::new(Kind::Note, super::entry::new_id(), 1_763_700_000);
    note.set(Field::Title, "Locker");
    note.set(Field::Notes, "Combination 00-00-00 (demo).\n\nThe spare batteries are on the top shelf, the charger below them. Lock it again before you leave the pit.");
    out.extend([wifi, pin, note]);
    out
}

const BITWARDEN: &str = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
,,login,Robot laptop,,,0,,driver,demo-laptop-1,\n\
,,login,CAD workspace,,,0,https://cad.example.org,designer,demo-cad-2,\n\
,,login,Scouting app,,,0,https://scout.example.org,scout,demo-scout-2026,\n";

/// Keys, each given time to land.
fn press(shell: &mut Shell, keys: &[Key]) {
    for key in keys {
        shell.key(*key);
        settle(shell, 30);
    }
    settle(shell, 50);
}

fn typed(shell: &mut Shell, text: &str) {
    for c in text.chars() {
        shell.key(Key::Char(c));
        settle(shell, 2);
    }
}

/// Frames until `done`, with the vault's threads given real time to work.
fn until(shell: &mut Shell, done: impl Fn(&Shell) -> bool) {
    let began = Instant::now();
    while !done(shell) && began.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(5));
        settle(shell, 2);
    }
    settle(shell, 60);
}

/// Frames for `ms` of real time (a check on its thread).
fn pause(shell: &mut Shell, ms: u64) {
    let began = Instant::now();
    until(shell, |_| began.elapsed() >= Duration::from_millis(ms));
}

/// The row of the list an entry is on (the list's first row is New item).
fn row_of(title: &str) -> usize {
    super::with(|entries| entries.iter().position(|e| e.name() == title)).flatten().map_or(0, |i| i + 1)
}

pub fn gallery(dir: &Path) {
    std::fs::create_dir_all(dir).expect("create the shots folder");
    let looks = [("catalyst", false), ("catalyst", true), ("bezel", false), ("bezel", true)];
    let sizes = [TextSize::Compact, TextSize::Standard, TextSize::Large];
    // (page, look, size) → the panel's pixels.
    let mut shots: Vec<(String, usize, usize, Vec<u32>)> = Vec::new();
    for (li, (look, light)) in looks.iter().enumerate() {
        for (si, size) in sizes.iter().enumerate() {
            let state = std::env::temp_dir().join(format!("catalyst-vault-shots-{}", std::process::id()));
            std::fs::remove_dir_all(&state).ok();
            std::fs::create_dir_all(state.join("files")).expect("a state folder");
            std::env::set_var("CATALYST_CONF", state.join("robot"));
            std::env::set_var("CATALYST_FILES", state.join("files"));
            super::lock();
            super::screen::first_recipe();
            let mut shell = crate::fullheight::shell(look, *size);
            shell.settings.screen_off = 0;
            shell.settings.light = *light;
            shell.apply_settings();
            settle(&mut shell, 60);
            let out = dir.join(format!("{look}-{}-{}", if *light { "light" } else { "dark" }, size.name()));
            std::fs::create_dir_all(&out).expect("a look's folder");
            let mut save = |shell: &mut Shell, name: &str| {
                shell.draw();
                crate::platform::sim::save_png(&shell.gfx.canvas, &out.join(format!("{name}.png")), 2);
                shots.push((name.to_owned(), li, si, shell.gfx.canvas.px.clone()));
            };
            use Key::*;
            // No passcode yet: one comes first.
            shell.show(Box::new(super::Vault::new()));
            settle(&mut shell, 90);
            save(&mut shell, "01-set-a-passcode-first");
            press(&mut shell, &[Back]);
            let (phc, _, _) = crate::passcode::make(PASSCODE).expect("a passcode");
            crate::passcode::keep(&phc).expect("kept");
            // Made with it.
            shell.show(Box::new(super::Vault::new()));
            settle(&mut shell, 90);
            save(&mut shell, "02-make-the-vault");
            typed(&mut shell, PASSCODE);
            shell.key(Enter);
            until(&mut shell, |_| super::is_open());
            // (A four-digit passcode: the page on what that means comes first.)
            if shell.top_title() == "Passcode" {
                press(&mut shell, &[Back]);
            }
            super::change(|entries| entries.extend(demo())).expect("the demo entries");
            super::flush();
            settle(&mut shell, 30);
            save(&mut shell, "03-list");
            press(&mut shell, &[Char('/'), Char('t'), Char('e'), Char('a')]);
            save(&mut shell, "04-search");
            press(&mut shell, &[Back]);
            // Locked, a wrong passcode, opened again.
            press(&mut shell, &[Char('l')]);
            save(&mut shell, "05-locked");
            typed(&mut shell, "1111");
            shell.key(Enter);
            pause(&mut shell, 700);
            save(&mut shell, "06-wrong-passcode");
            typed(&mut shell, PASSCODE);
            shell.key(Enter);
            until(&mut shell, |_| super::is_open());
            // A login: masked, shown, its code.
            let down = vec![Down; row_of("GitHub")];
            press(&mut shell, &down);
            press(&mut shell, &[Enter]);
            save(&mut shell, "07-entry");
            press(&mut shell, &[Down, Char(' ')]);
            save(&mut shell, "08-entry-password-shown");
            press(&mut shell, &[Down]);
            save(&mut shell, "09-entry-2fa-code");
            press(&mut shell, &[Back]);
            // An eight-digit code alone in its row.
            let to = row_of("FRC Events") as i64 - row_of("GitHub") as i64;
            let keys: Vec<Key> = (0..to.unsigned_abs()).map(|_| if to > 0 { Down } else { Up }).collect();
            press(&mut shell, &keys);
            press(&mut shell, &[Enter, Down, Down]);
            save(&mut shell, "10-entry-8-digit-code");
            press(&mut shell, &[Back]);
            // A note, a Wi-Fi network, a card's PIN.
            let keys = [vec![Home], vec![Down; row_of("Locker")], vec![Enter]].concat();
            press(&mut shell, &keys);
            save(&mut shell, "11-secure-note");
            let keys = [vec![Back, Home], vec![Down; row_of("Pit Wi-Fi")], vec![Enter]].concat();
            press(&mut shell, &keys);
            save(&mut shell, "12-wifi");
            let keys = [vec![Back, Home], vec![Down; row_of("Team debit card")], vec![Enter]].concat();
            press(&mut shell, &keys);
            save(&mut shell, "13-card-pin");
            // The editor, for a new login, and typing its password.
            press(&mut shell, &[Back, Home, Enter]);
            save(&mut shell, "14-new-item");
            press(&mut shell, &[Enter]);
            typed(&mut shell, "Robot laptop");
            press(&mut shell, &[Enter, Down, Down, Enter]);
            typed(&mut shell, "demo-pass");
            save(&mut shell, "15-typing-a-password");
            press(&mut shell, &[Back, Back, Back]);
            // The generator: characters, then words.
            press(&mut shell, &[Char('g')]);
            save(&mut shell, "16-generator");
            press(&mut shell, &[Right]);
            save(&mut shell, "17-generator-words");
            press(&mut shell, &[Back]);
            // Importing: the exports found, one read in, the offer to shred it.
            // (The newest file first: the CSV.)
            std::fs::write(state.join("files").join("aegis-codes.txt"), "otpauth://totp/Discord:pit-crew?secret=GEZDGNBVGY3TQOJQ&issuer=Discord\n").ok();
            std::thread::sleep(Duration::from_millis(20));
            std::fs::write(state.join("files").join("bitwarden_export.csv"), BITWARDEN).ok();
            press(&mut shell, &[Char('i')]);
            save(&mut shell, "18-import");
            press(&mut shell, &[Enter]);
            until(&mut shell, |s| s.top_title() == "Imported");
            save(&mut shell, "19-imported-shred-it");
            press(&mut shell, &[Enter]);
            // A backup's password, and how importing works: the rows after the files.
            let files = super::import::files(&crate::files::storage()).len().max(1);
            let keys = [vec![Home], vec![Down; files + 1], vec![Enter]].concat();
            press(&mut shell, &keys);
            typed(&mut shell, "a long backup phrase");
            save(&mut shell, "20-backup-password");
            press(&mut shell, &[Back, Back]);
            let keys = [vec![Home], vec![Down; files], vec![Enter]].concat();
            press(&mut shell, &keys);
            save(&mut shell, "21-how-importing-works");
            press(&mut shell, &[Back, Back, Back]);
            // Made with a short passcode: what that means.
            super::delete().ok();
            shell.show(Box::new(super::Vault::new()));
            settle(&mut shell, 60);
            typed(&mut shell, PASSCODE);
            shell.key(Enter);
            until(&mut shell, |s| s.top_title() == "Passcode");
            save(&mut shell, "22-a-short-passcode");
            press(&mut shell, &[Back, Back]);
            // A file changed on the card: it will not open, and may be set aside.
            super::lock();
            super::flush();
            let mut file = std::fs::read(super::path()).expect("the vault's file");
            let at = file.len() - 20;
            file[at] ^= 1;
            std::fs::write(super::path(), &file).expect("changed");
            shell.show(Box::new(super::Vault::new()));
            settle(&mut shell, 60);
            typed(&mut shell, PASSCODE);
            shell.key(Enter);
            pause(&mut shell, 700);
            save(&mut shell, "23-a-damaged-file");
            press(&mut shell, &[Enter]);
            save(&mut shell, "24-set-aside");
            press(&mut shell, &[Back]);
            super::lock();
            println!("24 pages in {}", out.display());
        }
    }
    sheets(dir, &shots, looks.len(), sizes.len());
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
        enc.write_header().expect("a png").write_image_data(&rgb).expect("written");
    }
}
