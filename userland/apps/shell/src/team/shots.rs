//! `--team-shots <dir>`: Team at an event, with no network: the welcome,
//! making a team, the invitation, joining by code (typing it, asking) and
//! from a QR code, the thread (and a share in it), writing, the members,
//! the calls, and the island saying an urgent call and a plain message;
//! in both tones at the three text sizes, each on its own and all of one
//! tone and size on a sheet.

use bezel::{Canvas, Color};

use super::crypto::TeamKey;
use super::engine::{Engine, Out, To};
use super::wire::{Body, ShareKind};
use crate::gfx::TextSize;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Screen, Shell};

const TEAM: &str = "5805 pit";

/// The thread as the pit might have it by Q31: Ana queuing the robot, a
/// battery asked for, Ben's note, Ana's board for Q31, Cam's all good.
/// This handheld is Thomas's; Dee has walked off.
fn demo() -> Engine {
    let key = TeamKey::from_bytes([0x5d; 32]);
    let names = ["Thomas", "Ana", "Ben", "Cam", "Dee"];
    let mut e: Vec<Engine> = names.iter().enumerate().map(|(i, n)| Engine::new(key.clone(), 10 + i as u64, n, 1 + i as u32)).collect();
    let now = super::now_ms();
    let wall = chrono::Utc::now().timestamp();
    // Everyone hears everyone (Dee only long ago).
    fn pass(e: &mut [Engine], from: usize, out: Out, now: u64) {
        for (to, bytes) in out {
            for t in 0..e.len() {
                if t != from && (to == To::All || To::Peer(e[t].me) == to) {
                    let _ = e[t].receive(&bytes, None, now);
                }
            }
        }
    }
    for i in 0..e.len() {
        let at = if i == 4 { now - 120_000 } else { now };
        let hello = e[i].hello().map(|h| vec![(To::All, h)]).unwrap_or_default();
        pass(&mut e, i, hello, at);
    }
    let said: [(usize, Body, bool, i64); 6] = [
        (1, Body::Call("robot to queue".into()), true, 840),
        (0, Body::Call("battery needed".into()), true, 610),
        (2, Body::Text("bumpers swapped to red, cart is by the pit door".into()), false, 420),
        (1, Body::Share { kind: ShareKind::Strategy, set: 31, part: 1, of: 1, label: "board Q31".into(), data: String::new() }, false, 300),
        (3, Body::Call("all good".into()), false, 120),
        (0, Body::Text("on our way to queue".into()), false, 40),
    ];
    for (who, body, urgent, ago) in said {
        let out = e[who].send(body, urgent, wall - ago, now);
        pass(&mut e, who, out, now);
    }
    // Ana and Ben have read it all; Cam not the last.
    for i in [1, 2, 3] {
        if i != 3 {
            e[i].read_all(now);
        }
        let hello = e[i].hello().map(|h| vec![(To::All, h)]).unwrap_or_default();
        pass(&mut e, i, hello, now);
    }
    let mut mine = e.remove(0);
    mine.read_all(now);
    mine.events.clear();
    mine
}

fn qr_text() -> String {
    let key = TeamKey::from_bytes([0x5d; 32]);
    format!("{}{} {TEAM}", super::QR_PREFIX, catalyst_companion_proto::hex(key.bytes()))
}

pub fn shots(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let own = std::env::temp_dir().join(format!("catalyst-team-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&own).ok();
    std::env::set_var("CATALYST_TEAM", &own);
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let typed = |shell: &mut Shell, words: &str| {
        for c in words.chars() {
            shell.key(Key::Char(c));
        }
    };
    for light in [false, true] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            let tone = if light { "light" } else { "dark" };
            let fresh = || {
                let mut shell = Shell::new(Robot::demo());
                shell.settings.light = light;
                shell.settings.text = size;
                shell.apply_settings();
                settle(&mut shell, 100);
                shell
            };
            let mut taken: Vec<Canvas> = Vec::new();
            let mut save = |shell: &mut Shell, what: &str| {
                shell.draw();
                let name = format!("{tone}-{}-{what}", size.name());
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 2);
                taken.push(shell.gfx.canvas.clone());
            };
            let page = |shell: &mut Shell, screen: Box<dyn Screen>| {
                shell.show(screen);
                settle(shell, 90);
            };
            // No team yet: the ways in.
            super::remove_demo();
            let mut shell = fresh();
            page(&mut shell, Box::new(super::TeamScreen::new()));
            save(&mut shell, "01-welcome");
            // Making one: its name typed.
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ui::CreatePage::new()));
            for _ in 0..8 {
                shell.key(Key::Backspace);
            }
            typed(&mut shell, "5805 pit crew");
            settle(&mut shell, 20);
            save(&mut shell, "02-create");
            // Its invitation: the QR code and the six digits.
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ui::InvitePage::shown(&qr_text(), TEAM, "482913", 263)));
            save(&mut shell, "03-invite-qr");
            // Joining with the code: typed, then asking.
            let mut shell = fresh();
            page(&mut shell, Box::new(super::JoinPage::by_code()));
            typed(&mut shell, "482 91");
            settle(&mut shell, 20);
            save(&mut shell, "04-join-code");
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ui::JoinPage::waiting("482913")));
            save(&mut shell, "05-join-asking");
            // From the QR code, read in the Camera: the name to join as.
            let mut shell = fresh();
            let (key, team) = super::read_qr(&qr_text()).expect("the team's code");
            page(&mut shell, Box::new(super::JoinPage::scanned(key, team)));
            save(&mut shell, "06-join-scanned");
            // In the team: the thread, its newest in focus, then the share.
            super::install_demo(demo(), TEAM);
            let mut shell = fresh();
            page(&mut shell, Box::new(super::TeamScreen::new()));
            save(&mut shell, "07-thread");
            shell.key(Key::Up);
            shell.key(Key::Up);
            settle(&mut shell, 40);
            save(&mut shell, "08-thread-share");
            // Writing.
            let mut shell = fresh();
            page(&mut shell, Box::new(super::TeamScreen::new()));
            typed(&mut shell, "be there in 2");
            settle(&mut shell, 60);
            save(&mut shell, "09-compose");
            // The members.
            let mut shell = fresh();
            page(&mut shell, Box::new(super::TeamScreen::new()));
            shell.key(Key::Tab);
            settle(&mut shell, 60);
            save(&mut shell, "10-members");
            // The calls.
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ui::CallsPage::new()));
            save(&mut shell, "11-calls");
            // The island: an urgent call over another app, and a message.
            let mut shell = fresh();
            let (index, ..) = crate::screens::builtin_apps().find(|(_, _, n)| *n == "Scout").expect("Scout is built in");
            let scout = crate::screens::open_builtin(index, &shell.robot);
            page(&mut shell, scout);
            shell.notify(crate::notify::Notification::new("Team", "groups", "Ana").body("robot to queue").urgent().action("builtin:Team"));
            settle(&mut shell, 36);
            save(&mut shell, "12-island-urgent");
            let mut shell = fresh();
            let scout = crate::screens::open_builtin(index, &shell.robot);
            page(&mut shell, scout);
            shell.notify(crate::notify::Notification::new("Team", "groups", "Ben").body("cart is by the pit door").action("builtin:Team"));
            settle(&mut shell, 36);
            save(&mut shell, "13-island-message");
            // All of one tone and size on a sheet, two across.
            let (cols, gap) = (2usize, 6i32);
            let rows = taken.len().div_ceil(cols);
            let mut sheet = Canvas::new(cols as i32 * (320 + gap) - gap, rows as i32 * (170 + gap) - gap);
            sheet.clear(Color::hex(0x808080));
            for (k, shot) in taken.iter().enumerate() {
                let (ox, oy) = ((k % cols) as i32 * (320 + gap), (k / cols) as i32 * (170 + gap));
                for y in 0..shot.h {
                    let from = (y * shot.w) as usize;
                    let to = ((oy + y) * sheet.w + ox) as usize;
                    sheet.px[to..to + shot.w as usize].copy_from_slice(&shot.px[from..from + shot.w as usize]);
                }
            }
            crate::platform::sim::save_png(&sheet, &dir.join(format!("sheet-{tone}-{}.png", size.name())), 2);
            println!("{tone} {}", size.name());
        }
    }
    super::remove_demo();
    std::fs::remove_dir_all(&own).ok();
}
