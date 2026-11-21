//! External apps: programs that are not part of the shell. They cost no
//! memory until opened and none after they are left.
//!
//! An app is a folder holding a file named `app`:
//!
//! ```text
//! name = Processes
//! icon = memory            (one of the built-in symbols; optional)
//! about = what is running  (the line under the name; optional)
//! run = top                (a shell command line, run from the app's folder)
//! screen = terminal        (or "own")
//! ```
//!
//! `screen = terminal` runs the program in the handheld's terminal, which is
//! all a script or a text program needs. `screen = own` hands it the panel
//! and the keys outright: it draws to the framebuffer named in `CATALYST_FB`
//! and reads the keyboard itself, and the shell stands aside until it ends.
//!
//! A script dropped straight into an apps folder (`check.sh`) is an app too,
//! with no folder or `app` file needed: it runs in the terminal.
//!
//! Either way, Ctrl-Q or Esc held for a second always comes back Home.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use bezel::{Align, Rect};

use crate::gfx::{Gfx, H, PAD, W};
use crate::platform::{Key, Platform};
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen, Settings};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Runs in the handheld's terminal.
    Terminal,
    /// Takes the panel and keyboard for itself.
    Own,
}

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub name: String,
    pub icon: String,
    pub about: String,
    /// A shell command line.
    pub run: String,
    /// The folder it is run from.
    pub dir: PathBuf,
    pub kind: Kind,
}

/// The folders apps are looked for in: those that come with the system, then
/// the user's own on the data partition.
pub fn folders() -> Vec<PathBuf> {
    if let Some(list) = std::env::var_os("CATALYST_APPS") {
        return std::env::split_paths(&list).collect();
    }
    if cfg!(target_os = "linux") {
        vec!["/usr/share/catalyst/apps".into(), "/data/apps".into()]
    } else {
        vec![std::env::temp_dir().join("catalyst-apps")]
    }
}

/// Read an `app` file. `None` if it names nothing to run.
pub fn parse(text: &str, dir: &Path) -> Option<App> {
    let mut app = App { name: String::new(), icon: String::new(), about: String::new(), run: String::new(), dir: dir.to_owned(), kind: Kind::Terminal };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "name" => app.name = value.chars().take(24).collect(),
            "icon" => app.icon = value.to_owned(),
            "about" => app.about = value.chars().take(40).collect(),
            "run" => app.run = value.to_owned(),
            "screen" => app.kind = if value == "own" { Kind::Own } else { Kind::Terminal },
            _ => {}
        }
    }
    if app.run.is_empty() {
        return None;
    }
    if app.name.is_empty() {
        app.name = dir.file_name()?.to_string_lossy().into_owned();
    }
    Some(finish(app))
}

/// Fill in what was left out.
fn finish(mut app: App) -> App {
    if !bezel::Icons::has(&app.icon) {
        app.icon = if app.kind == Kind::Terminal { "terminal" } else { "apps" }.into();
    }
    if app.about.is_empty() {
        app.about = if app.kind == Kind::Terminal { "terminal app" } else { "app" }.into();
    }
    app
}

/// A script lying loose in an apps folder.
fn loose_script(path: &Path) -> Option<App> {
    if path.extension()? != "sh" {
        return None;
    }
    let file = path.file_name()?.to_string_lossy().into_owned();
    Some(finish(App {
        name: path.file_stem()?.to_string_lossy().chars().take(24).collect(),
        icon: String::new(),
        about: "script".into(),
        // Through sh, so the script need not be marked executable (the
        // editor does not do that).
        run: format!("sh './{}'", file.replace('\'', "'\\''")),
        dir: path.parent()?.to_owned(),
        kind: Kind::Terminal,
    }))
}

/// Every app in the given folders, in name order within each folder.
pub fn find_in(folders: &[PathBuf]) -> Vec<App> {
    let mut apps = Vec::new();
    for folder in folders {
        let mut found: Vec<App> = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    parse(&std::fs::read_to_string(path.join("app")).ok()?, &path)
                } else {
                    loose_script(&path)
                }
            })
            .collect();
        found.sort_by_key(|app| app.name.to_lowercase());
        apps.extend(found);
    }
    apps
}

pub fn find() -> Vec<App> {
    find_in(&folders())
}

impl App {
    /// The screen that opens when the app's tile is chosen.
    pub fn open(&self) -> Box<dyn Screen> {
        match self.kind {
            Kind::Terminal => Box::new(crate::terminal::Terminal::running(&self.name, &self.run, self.dir.clone())),
            Kind::Own => Box::new(Handover { app: self.clone() }),
        }
    }
}

// -------------------------------------------------------------- handover ---

/// The page behind an app that draws for itself: it grows out of the tile,
/// and the program is started on top of it once it has landed. When the
/// program ends the page closes, from the program's last picture, back into
/// the tile.
pub struct Handover {
    app: App,
}

impl Screen for Handover {
    fn title(&self) -> &str {
        &self.app.name
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        // Keys only arrive here while the page grows (the program has every
        // key once it starts): Esc turns it back.
        if key.leaves() || key == Key::Back {
            return Nav::Pop;
        }
        Nav::Stay
    }

    /// Each time the page lands and shows alone: opened again, or asked back
    /// while it was closing, the program starts again. (The shell closes the
    /// page once the program ends, so it is not asked twice in a landing.)
    fn launch(&mut self) -> Option<App> {
        Some(self.app.clone())
    }

    /// The program ends when its page is left: nothing of it is kept to
    /// come back to, so a page left over can never stand in for it.
    fn keep_in_recents(&self) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let card = Rect::new(PAD, g.top(), W - 2.0 * PAD, H - g.top() - PAD);
        g.surface(card, 14.0, t.surface1);
        g.icon(&self.app.icon, true, card.cx(), card.cy() - ty.name.size - 4.0, 28.0, t.ink);
        g.text_at(card.cx(), card.cy() + 8.0, Align::Center, &self.app.name, ty.name, t.ink);
        g.text_at(card.cx(), card.cy() + 12.0 + ty.name.size, Align::Center, "ctrl-q or hold esc returns", ty.caption, t.dim);
    }
}

/// Start a program that draws for itself, with the panel and the look in use
/// passed along so it can match.
fn spawn(app: &App, settings: &Settings, framebuffer: Option<String>) -> std::io::Result<Child> {
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", &app.run]);
        c
    } else {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", &app.run]);
        c
    };
    command.current_dir(&app.dir);
    if let Some(fb) = framebuffer {
        command.env("CATALYST_FB", fb);
    }
    command
        .env("CATALYST_LIGHT", if settings.light { "1" } else { "0" })
        .env("CATALYST_CALM", if settings.calm { "1" } else { "0" })
        .env("CATALYST_ACCENT", settings.accent.key())
        .env("CATALYST_TEXT", settings.text.name().to_lowercase());
    #[cfg(target_os = "linux")]
    {
        // Its own process group, so the program and anything it started can
        // be stopped together.
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()
}

/// Ask the program and everything it started to stop; make whatever has not
/// within a second. The program ending is not enough: something it started
/// may have ignored the request, and would go on drawing on the panel.
fn stop(child: &mut Child) {
    #[cfg(target_os = "linux")]
    {
        let group = -(child.id() as i32);
        unsafe { libc::kill(group, libc::SIGTERM) };
        let asked = Instant::now();
        // Until nothing of the group is left, or the second is up.
        while asked.elapsed() < Duration::from_secs(1) {
            child.try_wait().ok();
            if unsafe { libc::kill(group, 0) } != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        unsafe { libc::kill(group, libc::SIGKILL) };
    }
    child.kill().ok();
    child.wait().ok();
}

/// Hand the panel to an app until it ends or the user leaves it. The shell
/// draws nothing meanwhile; it only watches the keys for the way out.
/// `Err` carries a line to tell the user.
pub fn run(app: &App, settings: &Settings, platform: &mut dyn Platform, canvas: &bezel::Canvas) -> Result<(), String> {
    let mut child = spawn(app, settings, platform.framebuffer()).map_err(|_| format!("could not start {}", app.name))?;
    let started = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            // Ending at once with an error is worth a word; later, the app has had its say.
            Ok(Some(_)) if started.elapsed() < Duration::from_secs(2) => break Err(format!("{} stopped with an error", app.name)),
            Ok(Some(_)) => break Ok(()),
            Ok(None) => {}
            Err(_) => break Err(format!("lost track of {}", app.name)),
        }
        match platform.poll(Duration::from_millis(100)) {
            Some(keys) if keys.iter().any(|k| k.leaves()) => {
                stop(&mut child);
                break Ok(());
            }
            Some(_) => {}
            None => {
                stop(&mut child);
                break Ok(());
            }
        }
        // A desktop window has to be fed to stay alive; the panel is left alone.
        if platform.present_always() {
            platform.present(canvas);
        }
    };
    // The program drew over whatever was showing.
    platform.invalidate();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_file_is_read_and_its_gaps_filled() {
        let dir = Path::new("/apps/clock");
        let app = parse("name = Clock\nicon = schedule\nabout = big digits\nrun = ./clock --big\nscreen = own\n", dir).unwrap();
        assert_eq!((app.name.as_str(), app.icon.as_str(), app.about.as_str()), ("Clock", "schedule", "big digits"));
        assert_eq!((app.run.as_str(), app.kind), ("./clock --big", Kind::Own));
        // Only `run` is required: the folder names it, and it gets a terminal.
        let bare = parse("run = top\n", Path::new("/apps/processes")).unwrap();
        assert_eq!((bare.name.as_str(), bare.kind, bare.icon.as_str(), bare.about.as_str()), ("processes", Kind::Terminal, "terminal", "terminal app"));
        // A symbol the shell does not carry falls back to one it does.
        assert_eq!(parse("run = x\nicon = no_such_symbol\nscreen = own", dir).unwrap().icon, "apps");
        // Nothing to run is not an app.
        assert!(parse("name = Empty\n", dir).is_none());
    }

    #[test]
    fn apps_are_found_in_folders_and_as_loose_scripts() {
        let root = std::env::temp_dir().join(format!("catalyst-apps-test-{}", std::process::id()));
        let (system, mine) = (root.join("system"), root.join("mine"));
        std::fs::create_dir_all(system.join("processes")).unwrap();
        std::fs::create_dir_all(mine.join("zebra")).unwrap();
        std::fs::create_dir_all(mine.join("not-an-app")).unwrap();
        std::fs::write(system.join("processes/app"), "name = Processes\nrun = top\n").unwrap();
        std::fs::write(mine.join("zebra/app"), "run = ./zebra\nscreen = own\n").unwrap();
        std::fs::write(mine.join("check it.sh"), "echo ok\n").unwrap();
        std::fs::write(mine.join("notes.txt"), "not a script\n").unwrap();
        let apps = find_in(&[system, mine.clone(), root.join("missing")]);
        // The system's first, then the user's by name; other files are passed over.
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Processes", "check it", "zebra"]);
        assert_eq!(apps[1].run, "sh './check it.sh'");
        assert_eq!((apps[1].dir.clone(), apps[1].about.as_str()), (mine, "script"));
        std::fs::remove_dir_all(&root).ok();
    }

    /// A program handed the screen runs with the look passed along, and
    /// ending by itself is not an error.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_program_is_started_told_the_look_and_stopped() {
        let dir = std::env::temp_dir().join(format!("catalyst-run-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = Settings { light: true, calm: false, text: crate::gfx::TextSize::Standard, accent: bezel::Accent::Blue, ..Settings::default() };
        let app = |run: &str| App { name: "T".into(), icon: "apps".into(), about: String::new(), run: run.into(), dir: dir.clone(), kind: Kind::Own };
        let mut child = spawn(&app("echo $CATALYST_FB $CATALYST_LIGHT $CATALYST_ACCENT $CATALYST_TEXT > seen"), &settings, Some("/dev/fb7".into())).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(std::fs::read_to_string(dir.join("seen")).unwrap().trim(), "/dev/fb7 1 blue standard");
        // One that would run for a minute, with a child of its own, is stopped at once, child and all.
        let mut child = spawn(&app("sleep 60 & echo $! > child; wait"), &settings, None).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let began = Instant::now();
        stop(&mut child);
        assert!(began.elapsed() < Duration::from_secs(3));
        let grandchild: i32 = std::fs::read_to_string(dir.join("child")).unwrap().trim().parse().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(unsafe { libc::kill(grandchild, 0) }, -1, "what the program started is still running");
        std::fs::remove_dir_all(&dir).ok();
    }
}
