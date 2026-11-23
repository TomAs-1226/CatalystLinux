//! `--strategy-shots <dir>`: the board on the demo robot's next match (Q31,
//! the team red with 1507 and 5254 against 3044, 6328 and 2337), empty and
//! with a full plan, each phase, playing, a path being drawn, a robot
//! picked up, a note being typed, the menu, the templates, the QR codes
//! and the keys; in both tones at the three text sizes, each shot on its
//! own and all of one tone and size on a sheet. The picture for Companion
//! is saved beside them.

use std::time::Duration;

use bezel::{Canvas, Color};

use super::board::{self, Board, Note, NoteKind, Phase, Route};
use crate::gfx::TextSize;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::Shell;

/// A plan for Q31 as a drive team might draw it: 5805 out to the fuel in
/// the middle and back to shoot, its partners to the depot and the
/// outpost; defense on 3044 in teleop; the climb. Written as the board
/// shows it (along from the team's wall, up from the foot), the team red.
pub fn plan(board: &mut Board) {
    use crate::field::{LENGTH, WIDTH};
    let at = |u: f32, v: f32| (LENGTH - u, WIDTH - v);
    let from = |b: &Board, phase: Phase, r: usize| b.end_of(phase, r).unwrap();
    let route = |b: &Board, phase: Phase, r: usize, rest: &[(f32, f32)]| {
        let before = match phase {
            Phase::Auto => b.start_of(Phase::Auto, r).unwrap(),
            Phase::Teleop => from(b, Phase::Auto, r),
            Phase::Endgame => from(b, Phase::Teleop, r),
        };
        Route { robot: r, points: std::iter::once(before).chain(rest.iter().map(|&(u, v)| at(u, v))).collect() }
    };
    let auto = vec![
        route(board, Phase::Auto, 0, &[(6.3, 5.6), (8.1, 4.4), (3.6, 3.1)]),
        route(board, Phase::Auto, 1, &[(1.1, 0.9), (3.0, 1.4)]),
        route(board, Phase::Auto, 2, &[(1.2, 6.0), (2.9, 6.8)]),
        route(board, Phase::Auto, 3, &[(10.4, 5.4), (8.9, 4.9)]),
        route(board, Phase::Auto, 4, &[(10.9, 7.2)]),
    ];
    board.layer_mut(Phase::Auto).paths = auto;
    board.layer_mut(Phase::Auto).notes = vec![
        Note { kind: NoteKind::Zone, robot: 0, at: at(8.1, 4.4), text: "fuel".into() },
        Note { kind: NoteKind::Label, robot: 0, at: at(3.6, 3.1), text: "0:04 shoot".into() },
    ];
    let teleop = vec![
        route(board, Phase::Teleop, 0, &[(8.4, 6.3), (4.2, 5.7), (8.6, 2.6), (3.8, 2.6)]),
        route(board, Phase::Teleop, 1, &[(9.6, 2.2), (11.7, 3.3)]),
        route(board, Phase::Teleop, 2, &[(1.2, 6.0), (3.4, 7.2)]),
        route(board, Phase::Teleop, 3, &[(12.2, 4.6)]),
    ];
    board.layer_mut(Phase::Teleop).paths = teleop;
    board.layer_mut(Phase::Teleop).notes = vec![Note { kind: NoteKind::Label, robot: 1, at: at(10.4, 2.0), text: "defense on 3044".into() }];
    let endgame = vec![
        route(board, Phase::Endgame, 0, &[(1.2, 3.75)]),
        route(board, Phase::Endgame, 1, &[(6.4, 1.5), (2.4, 2.0)]),
        route(board, Phase::Endgame, 2, &[(1.6, 5.1)]),
    ];
    board.layer_mut(Phase::Endgame).paths = endgame;
    board.layer_mut(Phase::Endgame).notes = vec![
        Note { kind: NoteKind::Zone, robot: 0, at: at(1.2, 3.75), text: "climb".into() },
        Note { kind: NoteKind::Label, robot: 0, at: at(2.2, 4.6), text: "climb by 0:20".into() },
    ];
}

pub fn shots(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let keys = |shell: &mut Shell, keys: &[Key]| {
        for k in keys {
            shell.key(*k);
        }
    };
    let typed = |shell: &mut Shell, words: &str| {
        for c in words.chars() {
            shell.key(Key::Char(c));
        }
    };
    let app = |shell: &mut Shell| {
        let (index, ..) = crate::screens::builtin_apps().find(|(_, _, n)| *n == "Strategy").expect("Strategy is built in");
        let screen = crate::screens::open_builtin(index, &shell.robot);
        shell.show(screen);
    };
    let home = board::home();
    let q31 = || {
        let view = crate::schedule::Schedule::demo().view().unwrap();
        let m = view.matches.iter().find(|m| m.label == "Q31").unwrap().clone();
        Board::of_match(&m, view.team)
    };
    let mut pictured = false;
    for light in [false, true] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            let tone = if light { "light" } else { "dark" };
            std::fs::remove_dir_all(&home).ok();
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
            // A fresh board, as the schedule fills it, the greeting gone.
            let mut shell = fresh();
            app(&mut shell);
            settle(&mut shell, 330);
            save(&mut shell, "01-empty");
            // What a frame of it costs with the cursor moving, against
            // painting the picture under it (once, then laid down).
            if !light && size == TextSize::Standard {
                let started = std::time::Instant::now();
                for k in 0..200 {
                    shell.key(Key::Char(if k % 40 < 20 { 'c' } else { 'z' }));
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                }
                let frame = started.elapsed().as_secs_f64() * 1000.0 / 200.0;
                println!("strategy on this computer: a frame with the cursor moving {frame:.2} ms");
                settle(&mut shell, 120);
            }
            // The full plan, kept as the drive team would have it, and two templates.
            let mut board = q31();
            plan(&mut board);
            board::save_board(&home, &board).unwrap();
            board::save_template(&home, &board.template("our standard auto")).unwrap();
            board::save_template(&home, &board.template("defense on 1507")).unwrap();
            if !pictured {
                pictured = true;
                let picture = super::picture(&mut shell.gfx, &board);
                super::write_png(&picture, &dir.join("picture-q31-strategy.png")).unwrap();
            }
            let mut shell = fresh();
            app(&mut shell);
            settle(&mut shell, 330);
            save(&mut shell, "02-plan-auto");
            keys(&mut shell, &[Key::Char('t')]);
            settle(&mut shell, 90);
            save(&mut shell, "03-plan-teleop");
            keys(&mut shell, &[Key::Char('e')]);
            settle(&mut shell, 90);
            save(&mut shell, "04-plan-endgame");
            // Playing auto, the robots on their way.
            keys(&mut shell, &[Key::Char('a')]);
            settle(&mut shell, 60);
            keys(&mut shell, &[Key::Char(' ')]);
            settle(&mut shell, 55);
            save(&mut shell, "05-playing");
            keys(&mut shell, &[Key::Char(' ')]);
            settle(&mut shell, 60);
            // 5254 drawing a path in teleop: two waypoints in, the next leg dashed.
            keys(&mut shell, &[Key::Char('t'), Key::Char('3'), Key::Char('p')]);
            keys(&mut shell, &[Key::Char('c'), Key::Char('c'), Key::Char('c'), Key::Char('c'), Key::Char('x'), Key::Char('x'), Key::Enter]);
            keys(&mut shell, &[Key::Char('c'), Key::Char('c'), Key::Char('c'), Key::Char('f'), Key::Char('f'), Key::Char('f'), Key::Enter]);
            keys(&mut shell, &[Key::Char('c'), Key::Char('c'), Key::Char('x'), Key::Right]);
            settle(&mut shell, 60);
            save(&mut shell, "06-drawing-a-path");
            keys(&mut shell, &[Key::Char('p'), Key::Char('u'), Key::Char('u')]);
            settle(&mut shell, 30);
            // 5805 picked up in auto (the board opens with the cursor on it) and carried.
            let mut shell = fresh();
            app(&mut shell);
            settle(&mut shell, 330);
            keys(&mut shell, &[Key::Enter, Key::Char('c'), Key::Char('c'), Key::Char('f')]);
            settle(&mut shell, 40);
            save(&mut shell, "07-carrying-a-robot");
            keys(&mut shell, &[Key::Back]);
            settle(&mut shell, 40);
            // A note being typed.
            keys(&mut shell, &[Key::Char('n')]);
            typed(&mut shell, "wait for 1507");
            settle(&mut shell, 20);
            save(&mut shell, "08-typing-a-note");
            keys(&mut shell, &[Key::Back]);
            // The menu, the templates, and the codes.
            keys(&mut shell, &[Key::Char('m')]);
            settle(&mut shell, 60);
            save(&mut shell, "09-menu");
            keys(&mut shell, &[Key::Down, Key::Enter]);
            settle(&mut shell, 60);
            save(&mut shell, "10-templates");
            keys(&mut shell, &[Key::Back]);
            settle(&mut shell, 60);
            keys(&mut shell, &[Key::Down, Key::Down, Key::Enter]);
            settle(&mut shell, 60);
            save(&mut shell, "11-share-qr");
            keys(&mut shell, &[Key::Right]);
            settle(&mut shell, 40);
            save(&mut shell, "12-share-qr-next");
            keys(&mut shell, &[Key::Back]);
            settle(&mut shell, 40);
            keys(&mut shell, &[Key::Back]);
            settle(&mut shell, 60);
            // Zoomed in on the team's side, and the keys.
            keys(&mut shell, &[Key::Char('=')]);
            settle(&mut shell, 60);
            save(&mut shell, "13-zoomed");
            keys(&mut shell, &[Key::Char('-'), Key::Char('?')]);
            settle(&mut shell, 60);
            save(&mut shell, "14-keys");
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
    std::thread::sleep(Duration::from_millis(50));
}
