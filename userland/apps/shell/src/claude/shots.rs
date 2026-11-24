//! `--claude-shots <dir>`: Claude with no laptop and no network, from a
//! made-up one: pairing (the ways in, a QR code read, the code typed), the
//! sessions (one working, one waiting on an answer), a conversation as it
//! streams, a tool's row opened, the composer, the permission sheet, a new
//! session's folders, the laptop out of reach, and the island telling of a
//! request over another app; in both tones at the three text sizes, each on
//! its own and all of one tone and size on a sheet.

use bezel::{Canvas, Color};
use catalyst_companion_proto::claude::{Invite, Key as PairKey};

use super::link::{self, Approval, Convo, Row, Session, State, Trouble};
use super::Paired;
use crate::gfx::TextSize;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Screen, Shell};

const ADDRESS: &str = "https://laptop.tail1234.ts.net:10000";
pub const SESSION: &str = "3f0c2a9e-1b2c-4d5e-8f90-a1b2c3d4e5f6";

fn paired() -> Paired {
    Paired { address: ADDRESS.into(), key: PairKey::from_bytes([0x5d; 32]), computer: "THOMAS-LAPTOP".into(), since: 0 }
}

/// The laptop's sessions as an evening's work might leave them.
pub fn sessions(now: i64) -> Vec<Session> {
    let s = |id: &str, title: &str, project: &str, ago: i64| Session { id: id.into(), title: title.into(), project: project.into(), cwd: format!("C:\\Users\\User\\{project}"), last: now - ago, mode: "ask".into() };
    vec![
        s(SESSION, "Camera timer counts down from 10", "CatalystLinux", 40),
        s("8a1d4c2e-0b9f-4c3a-9e7d-55aa66bb77cc", "Swerve odometry drifts on the turn", "FrcCatalyst", 1_500),
        s("c0ffee00-1234-4abc-8def-0123456789ab", "Team: keep the call order the same everywhere", "CatalystLinux", 9_000),
        s("deadbeef-5678-4abc-9def-0123456789ab", "Ladder auto: path to the second piece", "FrcCatalyst", 90_000),
        s("feedface-9abc-4abc-adef-0123456789ab", "README for the camera's scan mode", "CatalystLinux", 260_000),
    ]
}

fn row(id: &str, kind: &str, text: &str) -> Row {
    Row { id: id.into(), kind: kind.into(), text: text.into(), ..Row::default() }
}

fn tool(id: &str, icon: &str, text: &str, state: &str, detail: &str) -> Row {
    Row { id: id.into(), kind: "tool".into(), text: text.into(), icon: icon.into(), state: state.into(), detail: detail.into(), ..Row::default() }
}

/// The conversation open: the timer fixed, the tests about to run.
pub fn convo() -> Convo {
    let rows = vec![
        row("u1", "you", "the camera's self-timer starts from 3 even when it says 10, fix it"),
        row("c1", "claude", "The timer reads its length before the setting is saved, so it falls back to 3. I'll read it when the shutter is pressed instead."),
        tool("t1", "description", "read camera.rs lines 1880–1960", "ok", "userland/apps/shell/src/camera.rs"),
        tool(
            "t2",
            "edit",
            "edited camera.rs +3 −1",
            "ok",
            "userland/apps/shell/src/camera.rs\n− let secs = self.timer_secs;\n+ let secs = self.settings.timer.seconds();\n+ self.timer_secs = secs;\n+ self.say(format!(\"timer {secs} s\"));",
        ),
        row("c2", "claude", "Now the tests for the camera."),
        tool("t3", "terminal", "ran cargo test -p catalyst-shell camera", "run", "$ cargo test -p catalyst-shell camera"),
    ];
    Convo { session: Some(SESSION.into()), cwd: "C:\\Users\\User\\CatalystLinux".into(), project: "CatalystLinux".into(), title: "Camera timer counts down from 10".into(), rows, loaded: true, mode: "ask".into(), ..Convo::default() }
}

pub fn approval(now: i64) -> Approval {
    Approval {
        id: "a1".into(),
        session: SESSION.into(),
        what: "run a command".into(),
        line: "cargo test --profile sim -p catalyst-shell camera -- --nocapture".into(),
        place: "in CatalystLinux".into(),
        detail: "$ cargo test --profile sim -p catalyst-shell camera -- --nocapture".into(),
        until: now + 272,
    }
}

/// The state as the shots and the tests begin from: paired, the list in.
pub fn demo(s: &mut State) {
    let now = link::laptop_now();
    s.paired = Some(paired());
    s.sessions = Some(sessions(now));
    s.laptop_now = now;
    s.working.insert(SESSION.into(), "CatalystLinux".into());
    s.approvals.push(Approval { session: "8a1d4c2e-0b9f-4c3a-9e7d-55aa66bb77cc".into(), id: "a0".into(), ..approval(now) });
    s.projects = Some(vec![
        ("C:\\Users\\User\\CatalystLinux".into(), "CatalystLinux".into(), now - 40),
        ("C:\\Users\\User\\FrcCatalyst".into(), "FrcCatalyst".into(), now - 1_500),
        ("C:\\Users\\User\\Desktop\\frclibs".into(), "frclibs".into(), now - 400_000),
    ]);
}

pub fn shots(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let own = std::env::temp_dir().join(format!("catalyst-claude-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&own).ok();
    std::env::set_var("CATALYST_CLAUDE", &own);
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
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
            // Not paired: the ways in.
            link::install_demo(|_| {});
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ClaudeApp::new()));
            save(&mut shell, "01-welcome");
            // The laptop's QR code, read in the Camera.
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ui::PairPage::scanned(Invite { address: ADDRESS.into(), key: PairKey::from_bytes([0x5d; 32]), computer: "THOMAS-LAPTOP".into() })));
            save(&mut shell, "02-pair-scanned");
            // Its address and code typed.
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ui::PairPage::by_code()));
            for c in "laptop.tail1234.ts.net:10000".chars() {
                shell.key(Key::Char(c));
            }
            shell.key(Key::Enter);
            for c in "4829".chars() {
                shell.key(Key::Char(c));
            }
            settle(&mut shell, 20);
            save(&mut shell, "03-pair-code");
            // Paired: the sessions, one working, one waiting on an answer.
            link::install_demo(demo);
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ClaudeApp::new()));
            shell.key(Key::Down);
            settle(&mut shell, 30);
            save(&mut shell, "04-sessions");
            // A conversation, Claude working in it.
            link::install_demo(|s| {
                demo(s);
                s.convo = Some(convo());
            });
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ui::Conversation::new()));
            save(&mut shell, "05-conversation");
            // A tool's row opened: the edit.
            shell.key(Key::Up);
            shell.key(Key::Up);
            shell.key(Key::Up);
            shell.key(Key::Enter);
            settle(&mut shell, 40);
            save(&mut shell, "06-tool-opened");
            // Writing the next message.
            link::install_demo(|s| {
                demo(s);
                s.working.clear();
                let mut c = convo();
                c.rows.last_mut().expect("a row").state = "ok".into();
                c.rows.push(row("c3", "claude", "All 41 camera tests pass. The timer now counts down from whatever the setting says."));
                s.convo = Some(c);
            });
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ui::Conversation::new()));
            for c in "good. now make the countdown speak the last three seconds, and keep it quiet in calm mode".chars() {
                shell.key(Key::Char(c));
            }
            settle(&mut shell, 20);
            save(&mut shell, "07-composer");
            // The permission sheet.
            link::install_demo(|s| {
                demo(s);
                s.convo = Some(convo());
                s.approvals.push(approval(link::laptop_now()));
            });
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ui::Conversation::new()));
            settle(&mut shell, 60);
            save(&mut shell, "08-approval");
            // A new session's folders.
            link::install_demo(demo);
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ui::Projects::new()));
            save(&mut shell, "09-new-session");
            // The laptop asleep: the list kept, said along its top.
            link::install_demo(|s| {
                demo(s);
                s.trouble = Some(Trouble::Asleep);
            });
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ClaudeApp::new()));
            save(&mut shell, "10-asleep-list");
            // And with nothing fetched yet.
            link::install_demo(|s| {
                s.paired = Some(paired());
                s.trouble = Some(Trouble::Asleep);
            });
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ClaudeApp::new()));
            save(&mut shell, "11-unreachable");
            // Not paired any more, on the laptop's side.
            link::install_demo(|s| {
                s.paired = Some(paired());
                s.trouble = Some(Trouble::NotPaired);
            });
            let mut shell = fresh();
            page(&mut shell, Box::new(super::ClaudeApp::new()));
            save(&mut shell, "12-not-paired");
            // The island over another app: a request waiting, the live activity.
            link::install_demo(demo);
            let mut shell = fresh();
            let (index, ..) = crate::screens::builtin_apps().find(|(_, _, n)| *n == "Notes").expect("Notes is built in");
            let notes = crate::screens::open_builtin(index, &shell.robot);
            page(&mut shell, notes);
            let _task = crate::liveactivity::task(crate::liveactivity::Kind::Claude, "Claude", "approval waiting · CatalystLinux");
            shell.notify(crate::notify::Notification::new("Claude", super::ICON, "approval waiting").body("run a command · cargo test -p catalyst-shell camera").urgent().action("builtin:Claude"));
            settle(&mut shell, 36);
            save(&mut shell, "13-island-approval");
            // A reply finished while away.
            let mut shell = fresh();
            let notes = crate::screens::open_builtin(index, &shell.robot);
            page(&mut shell, notes);
            shell.notify(crate::notify::Notification::new("Claude", super::ICON, "reply ready").body("All 41 camera tests pass.").action("builtin:Claude"));
            settle(&mut shell, 36);
            save(&mut shell, "14-island-reply");
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
    std::fs::remove_dir_all(&own).ok();
}
