//! The Batteries app's pages from a made-up fleet, in both tones at each
//! text size (and in the Bezel theme at the standard size), with a sheet
//! of each page's six for looking them over, and the printable labels.
//! `catalyst-shell --battery-shots <dir>`.

use std::path::Path;

use super::health::{analyse, synthetic, Reading};
use super::{Battery, Charge, Fleet, Use, CHARGE_SECONDS, REST_SECONDS};
use crate::gfx::TextSize;
use crate::robot::Robot;
use crate::shell::{Screen, Shell};

/// A match's reading from a made-up log: (rest volts, mΩ, how hard, current logged, seed).
fn reading(rest: f64, mohm: f64, hard: f64, current: bool, seed: u64) -> Reading {
    analyse(&synthetic(rest, mohm, hard, current, seed)[..]).ok().flatten().expect("a reading")
}

/// An event's fleet: eight batteries in every state, one flagged to retire
/// (its resistance climbing, a brownout in Q14), one read by sag alone (a
/// robot that logged no current then), one new.
pub fn demo_fleet(now: i64) -> Fleet {
    let mut f = Fleet::default();
    let b = |id: &str, name: &str, year: u16, charge: Charge| Battery { id: id.into(), name: name.into(), year: Some(year), charge, reminder: None, added: now - 86_400 * 3 };
    f.batteries = vec![
        b("1", "1", 2026, Charge::Off { since: now - 2 * 3600 }),
        b("2", "2", 2026, Charge::Charging { since: now - 32 * 60 }),
        b("3", "3", 2025, Charge::Off { since: now - (REST_SECONDS - 6 * 60) }),
        b("4", "4", 2025, Charge::Used { since: now - 20 * 60 }),
        b("5", "5", 2024, Charge::Off { since: now - 5 * 3600 }),
        b("6", "6", 2023, Charge::Off { since: now - 3 * 3600 }),
        b("7", "7", 2026, Charge::Off { since: now - 4 * 3600 }),
        b("old-blue", "Old Blue", 2022, Charge::Charging { since: now - CHARGE_SECONDS - 5 * 60 }),
    ];
    // (battery, match, rest volts, mΩ, how hard, current logged)
    let matches: [(&str, &str, f64, f64, f64, bool); 15] = [
        ("old-blue", "Q1", 12.6, 24.0, 1.0, false),
        ("1", "Q1", 12.9, 16.0, 1.0, true),
        ("5", "Q3", 12.8, 19.0, 1.0, true),
        ("6", "Q3", 12.7, 22.0, 1.0, true),
        ("old-blue", "Q5", 12.6, 27.0, 1.0, false),
        ("1", "Q5", 12.9, 16.5, 1.0, true),
        ("5", "Q7", 12.8, 20.0, 1.0, true),
        ("6", "Q7", 12.7, 24.0, 1.0, true),
        ("old-blue", "Q9", 12.5, 30.0, 1.05, false),
        ("1", "Q9", 12.9, 17.0, 1.0, true),
        ("5", "Q12", 12.8, 21.0, 1.0, true),
        ("6", "Q12", 12.6, 28.0, 1.05, true),
        ("6", "Q14", 12.5, 31.0, 1.35, true),
        ("1", "Q22", 12.9, 16.0, 1.1, true),
        ("4", "Q22", 12.8, 18.0, 1.0, true),
    ];
    let mut at = now - 86_400;
    for (k, (id, label, rest, mohm, hard, current)) in matches.into_iter().enumerate() {
        at += 3600;
        let r = reading(rest, mohm, hard, current, 7 + k as u64);
        f.uses.push(Use { battery: id.into(), label: label.into(), at, before: Charge::Off { since: at - 3600 }, log: Some(format!("FRC_20261115_1{k:02}000_NYRO_{label}.wpilog")), reading: Some(r) });
    }
    // The island has asked about the demo's Q31 already: its pages show
    // with their titles.
    f.asked.push("Q31".into());
    f
}

/// Each page shot: its name, and how to open it.
type Page = (&'static str, fn(&Robot) -> Box<dyn Screen>);

pub fn run(dir: &Path) {
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let now = super::now();
    let pages: [Page; 7] = [
        ("1-fleet", |_| Box::new(super::app::Fleet::new())),
        ("2-battery", |_| Box::new(super::app::Detail::new("1"))),
        ("3-battery-retire", |_| Box::new(super::app::Detail::new("6"))),
        ("4-battery-sag", |_| Box::new(super::app::Detail::new("old-blue"))),
        ("5-pick", |robot| super::app::open(robot)),
        ("6-new", |_| Box::new(super::app::Add::typed("Rusty", true))),
        ("7-fleet-empty", |_| Box::new(super::app::Fleet::new())),
    ];
    let looks: [(&str, bool, bool, TextSize); 8] = [
        ("dark-compact", false, false, TextSize::Compact),
        ("dark-standard", false, false, TextSize::Standard),
        ("dark-large", false, false, TextSize::Large),
        ("light-compact", true, false, TextSize::Compact),
        ("light-standard", true, false, TextSize::Standard),
        ("light-large", true, false, TextSize::Large),
        ("bezel-dark-standard", false, true, TextSize::Standard),
        ("bezel-light-standard", true, true, TextSize::Standard),
    ];
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
        shell.draw();
    };
    let fresh = |light: bool, bezel: bool, size: TextSize| {
        let mut shell = Shell::new(Robot::demo());
        shell.settings.light = light;
        if bezel {
            (shell.settings.theme, shell.settings.wallpaper, shell.settings.accent) = (crate::look::Theme::Bezel, crate::look::Wallpaper::Glow, bezel::Accent::Orange);
        }
        shell.settings.text = size;
        shell.apply_settings();
        shell
    };
    for (name, open) in pages {
        let mut pictures = Vec::new();
        for (look, light, bezel, size) in looks {
            super::pretend(if name == "7-fleet-empty" { Fleet::default() } else { demo_fleet(now) });
            let mut shell = fresh(light, bezel, size);
            settle(&mut shell, 20);
            // The demo's schedule as the shell's own robot has it.
            let page = open(&Robot::demo());
            shell.show(page);
            settle(&mut shell, 90);
            let file = dir.join(format!("{name}-{look}.png"));
            crate::platform::sim::save_png(&shell.gfx.canvas, &file, 3);
            pictures.push(shell.gfx.canvas.px.clone());
        }
        sheet(&pictures, &dir.join(format!("sheet-{name}.png")));
        println!("{name}");
    }

    // The island asking, as the queue nears with no battery picked.
    let mut pictures = Vec::new();
    for (look, light, bezel, size) in looks {
        let mut fleet = demo_fleet(now);
        fleet.asked.clear();
        super::pretend(fleet);
        // The shell's own watch asks, on its first look.
        let mut shell = fresh(light, bezel, size);
        settle(&mut shell, 60);
        assert!(super::read(|f| f.asked.iter().any(|a| a == "Q31")), "the island asked about Q31");
        crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("0-island-asks-{look}.png")), 3);
        pictures.push(shell.gfx.canvas.px.clone());
    }
    sheet(&pictures, &dir.join("sheet-0-island-asks.png"));
    println!("0-island-asks");

    // The labels, as Companion downloads them.
    let fleet = demo_fleet(now);
    let mut batteries = fleet.batteries.clone();
    batteries.sort_by(|a, b| super::natural(&a.name, &b.name));
    let file = std::fs::File::create(dir.join("labels.png")).expect("labels");
    super::label::write(&batteries, Some(5805), std::io::BufWriter::new(file)).expect("the labels");
    println!("labels");
    super::pretend(Fleet::default());
}

/// The pictures side by side, two across (dark, light), each twice its size.
fn sheet(pictures: &[Vec<u32>], path: &Path) {
    const S: usize = 2;
    let (w, h) = (crate::gfx::W as usize, crate::gfx::H as usize);
    let (across, gap) = (2, 8);
    let down = pictures.len().div_ceil(across);
    let (sw, sh) = (across * w * S + (across + 1) * gap, down * h * S + (down + 1) * gap);
    let mut canvas = bezel::Canvas::new(sw as i32, sh as i32);
    canvas.clear(bezel::Color::hex(0x808080));
    // Dark in the left column, light in the right; the Bezel pair last.
    let order: Vec<usize> = if pictures.len() == 8 { vec![0, 3, 1, 4, 2, 5, 6, 7] } else { (0..pictures.len()).collect() };
    for (slot, &i) in order.iter().enumerate() {
        let (ox, oy) = (gap + (slot % across) * (w * S + gap), gap + (slot / across) * (h * S + gap));
        for y in 0..h * S {
            for x in 0..w * S {
                canvas.px[(oy + y) * sw + ox + x] = pictures[i][(y / S) * w + x / S];
            }
        }
    }
    crate::platform::sim::save_png(&canvas, path, 1);
}
