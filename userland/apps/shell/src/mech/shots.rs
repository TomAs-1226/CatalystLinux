//! Mech's pages, dark and light, at each text size (`--mech-shots <dir>`):
//! `<n>-<page>-<dark|light>-<size>.png`, twice the panel's size.

use std::path::Path;

use super::screen::Form;
use super::{Calc, Mech, MechApp};
use crate::gfx::TextSize;
use crate::shell::{Screen, Shell};

pub fn shots(dir: &Path) {
    std::fs::create_dir_all(dir).expect("the shots folder");
    let state = std::env::temp_dir().join(format!("catalyst-mech-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&state).ok();
    std::fs::create_dir_all(&state).expect("a state folder");
    std::env::set_var("CATALYST_CONF", state.join("robot"));
    let mut wrist = Mech::new(Calc::Arm);
    wrist.name = "Wrist".into();
    super::save(&wrist).expect("kept");
    let mut shooter = Mech::new(Calc::Flywheel);
    shooter.name = "Shooter 2026".into();
    super::save(&shooter).expect("kept");
    let pages: Vec<(&str, fn() -> Box<dyn Screen>)> = vec![
        ("1-list", || Box::new(MechApp::new())),
        ("2-ratio", || Box::new(Form::new(Mech::new(Calc::Ratio)).at(2, None))),
        ("3-ratio-typing", || Box::new(Form::new(Mech::new(Calc::Ratio)).at(3, Some("18:3")))),
        ("4-drive", || Box::new(Form::new(Mech::new(Calc::Drive)))),
        ("5-drive-results", || Box::new(Form::new(Mech::new(Calc::Drive)).at(14, None))),
        ("6-arm", || Box::new(Form::new(Mech::new(Calc::Arm)).at(6, None))),
        ("7-elevator", || Box::new(Form::new(Mech::new(Calc::Elevator)).at(4, None))),
        ("8-flywheel", || Box::new(Form::new(Mech::new(Calc::Flywheel)).at(3, None))),
        ("9-keep-as", || Box::new(Form::new(Mech::new(Calc::Flywheel)).naming("Shooter 2026"))),
    ];
    for light in [false, true] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            for (name, page) in &pages {
                let mut shell: Shell = crate::fullheight::shell("catalyst", size);
                shell.settings.light = light;
                shell.apply_settings();
                crate::fullheight::settle(&mut shell, 30);
                shell.show(page());
                crate::fullheight::settle(&mut shell, 90);
                shell.draw();
                let file = format!("{name}-{}-{}.png", if light { "light" } else { "dark" }, size.name());
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(file), 2);
            }
        }
    }
    std::fs::remove_dir_all(&state).ok();
    println!("Mech's pages in {}", dir.display());
}
