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
//!
//! The app file is read by `appkg` (the catalogue's format: an id, a
//! version, what it uses, its credit and licence besides the above). An
//! app may carry its own icon (`icon = icon.svg`, a Material Symbol in its
//! 960-unit box), drawn as the shell's own are. An app is told the whole
//! look in use and given a folder of its own on `/data` that outlives
//! updates (`CATALYST_APP_DATA`): what Catalyst Kit reads.

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
    /// Its id (the folder's name unless the app file says), its version
    /// (empty if it does not say) and what it says it uses.
    pub id: String,
    pub version: String,
    pub permissions: Vec<appkg::Permission>,
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
    let m = appkg::Manifest::parse(text);
    if m.run.is_empty() {
        return None;
    }
    let folder = dir.file_name()?.to_string_lossy().into_owned();
    let id = if appkg::valid_id(&m.id) { m.id.clone() } else { folder.clone() };
    let mut app = App {
        name: if m.name.is_empty() { folder } else { m.name.clone() },
        icon: m.icon.clone(),
        about: m.about.clone(),
        run: m.run.clone(),
        dir: dir.to_owned(),
        kind: if m.screen == appkg::Screen::Own { Kind::Own } else { Kind::Terminal },
        id,
        version: m.version.clone(),
        permissions: m.permissions.clone(),
    };
    // Its own icon, carried beside the app file: drawn by the name `app:<id>`.
    if app.icon.ends_with(".svg") && !app.icon.contains('/') {
        let own = format!("app:{}", app.id);
        let path = std::fs::read_to_string(dir.join(&app.icon)).ok().and_then(|svg| bezel::icons::svg_path(&svg));
        let registered = path.is_some_and(|path| bezel::icons::register(&own, path));
        app.icon = if registered { own } else { String::new() };
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
    let stem: String = path.file_stem()?.to_string_lossy().into_owned();
    Some(finish(App {
        name: stem.chars().take(24).collect(),
        icon: String::new(),
        about: "script".into(),
        // Through sh, so the script need not be marked executable (the
        // editor does not do that).
        run: format!("sh './{}'", file.replace('\'', "'\\''")),
        dir: path.parent()?.to_owned(),
        kind: Kind::Terminal,
        id: stem.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect(),
        version: String::new(),
        permissions: Vec::new(),
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

/// An app's own folder for what it keeps (scores, settings): outside its
/// app folder, which an update replaces whole, so it outlives updates.
/// Made when the app first starts; removing the app leaves it.
pub fn data_dir(id: &str) -> PathBuf {
    let base = if cfg!(target_os = "linux") && Path::new("/data").is_dir() { PathBuf::from("/data/catalyst/app-data") } else { crate::config_dir().join("app-data") };
    let dir = base.join(if appkg::valid_id(id) { id } else { "app" });
    std::fs::create_dir_all(&dir).ok();
    dir
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
    // The rest of the look, so an app drawn with Catalyst Kit wears it whole;
    // who it is, and its own folder on /data, kept when it is updated.
    let permissions: Vec<&str> = app.permissions.iter().map(|p| p.name()).collect();
    command
        .env("CATALYST_THEME", settings.theme.name())
        .env("CATALYST_WALLPAPER", settings.wallpaper.name())
        .env("CATALYST_CORNERS", settings.corners.name())
        .env("CATALYST_LETTERS", settings.letters.name())
        .env("CATALYST_WIDTH", settings.width.name())
        .env("CATALYST_CLOCK24", if settings.clock24 { "1" } else { "0" })
        .env("CATALYST_ISLAND", settings.island.name())
        .env("CATALYST_APP_ID", &app.id)
        .env("CATALYST_APP_NAME", &app.name)
        .env("CATALYST_APP_DATA", data_dir(&app.id))
        .env("CATALYST_PERMISSIONS", permissions.join(","));
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
        // The catalogue's words: an id, a version, what it uses.
        let full = parse("id = big-clock\nname = Clock\nversion = 1.2.0\npermissions = network, speaker\nrun = ./clock\n", dir).unwrap();
        assert_eq!((full.id.as_str(), full.version.as_str()), ("big-clock", "1.2.0"));
        assert_eq!(full.permissions, [appkg::Permission::Network, appkg::Permission::Speaker]);
        assert_eq!(bare.id, "processes", "the folder names it when the file does not");
    }

    #[test]
    fn an_app_carries_its_own_icon() {
        let dir = std::env::temp_dir().join(format!("catalyst-app-icon-{}", std::process::id())).join("squares");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("icon.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path d="M160-160v-640h640v640H160Z"/></svg>"#).unwrap();
        let app = parse("name = Squares\nicon = icon.svg\nrun = ./squares\nscreen = own\n", &dir).unwrap();
        assert_eq!(app.icon, "app:squares");
        assert!(bezel::Icons::has("app:squares"));
        // One drawn in another box is not drawn wrong: the plain symbol stands in.
        std::fs::write(dir.join("icon.svg"), r#"<svg viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>"#).unwrap();
        let other_box = parse("name = Squares\nicon = icon.svg\nrun = ./squares\nscreen = own\n", &dir).unwrap();
        assert_eq!(other_box.icon, "apps");
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
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
        let app = |run: &str| App {
            name: "T".into(),
            icon: "apps".into(),
            about: String::new(),
            run: run.into(),
            dir: dir.clone(),
            kind: Kind::Own,
            id: "t".into(),
            version: "1.0.0".into(),
            permissions: vec![appkg::Permission::Network],
        };
        let mut child = spawn(&app("echo $CATALYST_FB $CATALYST_LIGHT $CATALYST_ACCENT $CATALYST_TEXT > seen"), &settings, Some("/dev/fb7".into())).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(std::fs::read_to_string(dir.join("seen")).unwrap().trim(), "/dev/fb7 1 blue standard");
        // The whole look and the app's own folder, as Catalyst Kit reads them.
        let mut child = spawn(&app("echo $CATALYST_THEME $CATALYST_CORNERS $CATALYST_APP_ID $CATALYST_PERMISSIONS ${CATALYST_APP_DATA##*/} > seen"), &settings, None).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(std::fs::read_to_string(dir.join("seen")).unwrap().trim(), format!("{} {} t network t", settings.theme.name(), settings.corners.name()));
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
