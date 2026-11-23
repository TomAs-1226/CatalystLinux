//! Every screen at the panel's whole height: the gallery that shows each one
//! at every text size in both looks (`--full-height <dir>`), and the test
//! that a list's viewport runs to the bottom of the panel unless the screen
//! says what is drawn along it (a hint line, a row of buttons).

#![cfg(not(target_arch = "aarch64"))]

use crate::gfx::TextSize;
use crate::look::Theme;
use crate::platform::Key;
use crate::robot::Robot;
use crate::screens;
use crate::shell::{Screen, Shell};

/// How a stop's screen is reached.
#[derive(Clone, Copy)]
pub enum Open {
    /// A built-in app, by name, opened straight away.
    App(&'static str),
    /// A place in Settings, by its name in search.
    Setting(&'static str),
    /// Something over the screen rather than a screen of its own.
    Over(fn(&mut Shell)),
}

/// One screen to look at: its name, how it is opened, and the keys pressed
/// once it is (to reach a page inside it).
pub struct Stop {
    pub name: String,
    pub open: Open,
    pub keys: Vec<Key>,
}

fn stop(name: &str, open: Open, keys: &[Key]) -> Stop {
    Stop { name: name.to_owned(), open, keys: keys.to_vec() }
}

/// Every built-in app's first page, every section of Settings, the pages
/// inside them that are lists, and what comes over the screen.
pub fn stops() -> Vec<Stop> {
    use Key::*;
    let mut all: Vec<Stop> = screens::builtin_apps()
        // Web is lynx in a terminal: the computer may not have it.
        .filter(|(_, _, name)| *name != "Web")
        .map(|(_, _, name)| stop(&format!("app-{}", slug(name)), Open::App(name), &[]))
        .collect();
    all.extend(screens::setting_sections().into_iter().filter(|(_, _, parent)| parent.is_none()).map(|(_, name, _)| stop(&format!("settings-{}", slug(name)), Open::Setting(name), &[])));
    let licences = crate::about::row_of("Licences").unwrap_or(6);
    all.extend([
        stop("logs-match", Open::App("Logs"), &[Enter]),
        stop("robot-field", Open::App("Robot"), &[Right]),
        stop("logs-trends", Open::App("Logs"), &[Up, Enter]),
        stop("tables-elevator", Open::App("Tables"), &[Down, Down, Enter]),
        stop("files-editor", Open::App("Files"), &[Down, Down, Enter]),
        stop("monitor-processes", Open::App("Monitor"), &[Right]),
        stop("net-tools-find", Open::App("Net tools"), &[Enter]),
        stop("pit-checklist", Open::App("Pit"), &[Enter]),
        stop("remote-buttons", Open::App("Remote"), &[Enter, Down]),
        stop("clock-world", Open::App("Clock"), &[Right, Right]),
        stop("weather-week", Open::App("Weather"), &[Down, Down]),
        stop("reminders-upcoming", Open::App("Reminders"), &[Right]),
        stop("units-torque", Open::App("Units"), &[Enter]),
        stop("settings-network-wifi", Open::Setting("Network"), &[Enter]),
        stop("settings-help-shortcuts", Open::Setting("Help"), &[Down, Enter]),
        stop("settings-about-licences", Open::Setting("About this handheld"), &vec![Down; licences].into_iter().chain([Enter]).collect::<Vec<_>>()),
        stop("settings-update-whats-new", Open::Setting("System update"), &[Down, Down, Enter]),
        stop("over-control-centre", Open::Over(|shell| shell.key(Key::Ctrl(' '))), &[]),
        stop("over-notifications", Open::Over(|shell| shell.key(Key::Ctrl(crate::platform::NOTIFICATIONS))), &[]),
        stop("over-spotlight", Open::Over(|shell| shell.key(Key::Ctrl('\n'))), &[Char('s')]),
        stop("over-recent-apps", Open::Over(recents), &[]),
        stop("over-setup", Open::Over(Shell::start_setup), &[]),
        stop("over-setup-wifi", Open::Over(Shell::start_setup), &[Enter]),
    ]);
    all
}

/// A few apps used and left, then the recent apps brought up.
fn recents(shell: &mut Shell) {
    use Key::*;
    for key in [Enter, Back, Right, Right, Enter, Back, Left, Enter] {
        shell.key(key);
        settle(shell, 70);
    }
    shell.key(Ctrl('\t'));
}

fn slug(name: &str) -> String {
    name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-")
}

pub fn settle(shell: &mut Shell, frames: usize) {
    for _ in 0..frames {
        if shell.tick(1.0 / 60.0) {
            shell.draw();
        }
    }
}

/// A shell on the demo robot in a look ("catalyst" or "bezel") and at a
/// text size, settled on Home.
pub fn shell(look: &str, size: TextSize) -> Shell {
    let mut shell = Shell::new(Robot::demo());
    let s = &mut shell.settings;
    if look == "bezel" {
        (s.theme, s.wallpaper, s.accent) = (Theme::Bezel, crate::look::Wallpaper::Glow, bezel::Accent::Orange);
    }
    s.text = size;
    shell.apply_settings();
    settle(&mut shell, 100);
    shell
}

/// Open a stop's screen on a shell settled on Home, and press its keys.
pub fn open(shell: &mut Shell, stop: &Stop) {
    match stop.open {
        Open::App(name) => {
            let (index, ..) = screens::builtin_apps().find(|(_, _, n)| *n == name).expect("a built-in app");
            let screen: Box<dyn Screen> = screens::open_builtin(index, &shell.robot);
            shell.show(screen);
        }
        Open::Setting(name) => {
            let (index, ..) = screens::setting_sections().into_iter().find(|(_, n, _)| *n == name).expect("a place in Settings");
            shell.show(screens::open_setting(index));
        }
        Open::Over(open) => open(shell),
    }
    settle(shell, 90);
    for key in &stop.keys {
        shell.key(*key);
        settle(shell, 60);
    }
    settle(shell, 60);
}

/// Every stop in one look at one text size, as the panel shows it (one
/// pixel a pixel), into `dir/<look>-<size>/<stop>.png`.
pub fn gallery(dir: &std::path::Path, look: &str, size: TextSize) {
    let out = dir.join(format!("{look}-{}", size.name()));
    std::fs::create_dir_all(&out).expect("create the screenshot folder");
    let stops = stops();
    for stop in &stops {
        let mut shell = shell(look, size);
        open(&mut shell, stop);
        shell.draw();
        crate::platform::sim::save_png(&shell.gfx.canvas, &out.join(format!("{}.png", stop.name)), 1);
    }
    println!("{} screens in {}", stops.len(), out.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bezel keeps the foot of its wall panels free for a dock; the handheld
    /// has none. So a list's viewport runs to the panel's bottom (its rows
    /// fading out under it), unless the screen draws something along the
    /// foot (a hint line, a row of buttons): then it runs to just that.
    #[test]
    fn every_list_runs_to_the_foot_of_the_panel_or_to_what_is_drawn_there() {
        crate::shell::tests::sandbox();
        let mut wrong = Vec::new();
        let mut lists = 0;
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            for stop in stops().iter().filter(|s| !matches!(s.open, Open::Over(_))) {
                let mut shell = shell("catalyst", size);
                open(&mut shell, stop);
                shell.draw();
                let g = &shell.gfx;
                for &(top, bottom) in &g.viewports {
                    lists += 1;
                    if (bottom - g.foot).abs() > 0.5 {
                        wrong.push(format!("{} at {}: the list from {top} ends at {bottom}, the foot is at {}", stop.name, size.name(), g.foot));
                    }
                }
            }
        }
        assert!(lists > 100, "only {lists} lists were drawn");
        assert!(wrong.is_empty(), "{} lists stop short of the foot or run under it:\n{}", wrong.len(), wrong.join("\n"));
    }
}
