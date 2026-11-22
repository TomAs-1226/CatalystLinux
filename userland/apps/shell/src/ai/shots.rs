//! The Smarter Assistant's screens, to look at (`--assistant-shots <dir>`):
//! a task of several steps under way and done, a briefing, the memory page,
//! in both looks at each text size; then, at the usual size, the choices,
//! a change asked about first, and the briefing made offline.
//!
//! The simulator's scripted service answers, with the demo robot, in a
//! settings folder of the run's own.

use std::path::Path;
use std::time::Duration;

use crate::gfx::TextSize;
use crate::look::Theme;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::Shell;

const EXAMPLE: &str = "download my Pit Mix playlist and remind me to swap the battery at queueing for Q42";

fn settle(shell: &mut Shell, frames: usize) {
    for _ in 0..frames {
        if shell.tick(1.0 / 60.0) {
            shell.draw();
        }
    }
}

/// Real time passing (the simulator's music search waits for it), with frames.
fn wait(shell: &mut Shell, ms: u64) {
    for _ in 0..ms / 20 {
        std::thread::sleep(Duration::from_millis(20));
        settle(shell, 2);
    }
}

fn typed(shell: &mut Shell, text: &str) {
    for c in text.chars() {
        shell.key(Key::Char(c));
    }
    settle(shell, 6);
}

fn fresh(look: &str, size: TextSize) -> Shell {
    crate::music::forget_downloads_for_shots();
    std::fs::remove_file(crate::reminders::path()).ok();
    crate::reminders::with(|s| s.book = crate::reminders::Book::open(crate::reminders::path()));
    let memory = super::memory::path();
    std::fs::remove_file(&memory).ok();
    for fact in ["The robot is called Numbers", "The team is 5805", "Likes short answers"] {
        super::memory::save(&memory, fact, "").ok();
    }
    std::fs::remove_dir_all(crate::config_dir().join("assistant-chats")).ok();
    let mut shell = Shell::new(Robot::demo());
    if look == "bezel" {
        let s = &mut shell.settings;
        (s.theme, s.wallpaper, s.accent) = (Theme::Bezel, crate::look::Wallpaper::Glow, bezel::Accent::Orange);
    }
    shell.settings.text = size;
    shell.apply_settings();
    settle(&mut shell, 100);
    shell
}

pub fn run(dir: &Path) {
    // A settings folder of this run's own: the keys, memory and reminders it writes.
    let own = std::env::temp_dir().join(format!("catalyst-assistant-shots-{}", std::process::id()));
    std::fs::create_dir_all(&own).ok();
    std::env::set_var("CATALYST_CONF", own.join("robot"));
    std::env::set_var("CATALYST_LOGS", own.join("logs"));
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let mut keys = crate::keys::Keys::default();
    keys.set("openai", "sk-proj-demo-7f3a");
    keys.save().ok();
    let save = |shell: &mut Shell, name: &str| {
        eprintln!("DBG save {name} {:?}", std::time::Instant::now());
        shell.draw();
        crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    };
    let mut missing = std::collections::BTreeSet::new();
    for look in ["catalyst", "bezel"] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            let tag = format!("{look}-{}", size.name());
            let mut shell = fresh(look, size);
            shell.show(Box::new(super::Assistant::new()));
            settle(&mut shell, 90);
            // The task of two steps: asked, the download under way, then the
            // reminder done and the model thinking, then the answer.
            typed(&mut shell, EXAMPLE);
            shell.key(Key::Enter);
            // The service's turn (0.9 s), then the download found on its thread.
            settle(&mut shell, 62);
            settle(&mut shell, 16);
            save(&mut shell, &format!("{tag}-01-task-under-way"));
            // Its thread done (real time), then the reminder a beat later,
            // and the service asked again.
            std::thread::sleep(Duration::from_millis(1200));
            settle(&mut shell, 26);
            save(&mut shell, &format!("{tag}-02-task-thinking"));
            settle(&mut shell, 110);
            save(&mut shell, &format!("{tag}-03-task-done"));
            // A briefing, the next match's (it is soon).
            shell.key(Key::Ctrl('n'));
            settle(&mut shell, 20);
            typed(&mut shell, "brief me");
            shell.key(Key::Enter);
            settle(&mut shell, 60);
            wait(&mut shell, 300);
            settle(&mut shell, 120);
            save(&mut shell, &format!("{tag}-04-briefing"));
            // What it remembers.
            shell.key(Key::Tab);
            settle(&mut shell, 60);
            for _ in 0..7 {
                shell.key(Key::Down);
                settle(&mut shell, 4);
            }
            settle(&mut shell, 40);
            if size == TextSize::Standard {
                save(&mut shell, &format!("{tag}-05-choices"));
            }
            shell.key(Key::Enter);
            settle(&mut shell, 70);
            save(&mut shell, &format!("{tag}-06-memory"));
            shell.key(Key::Down);
            settle(&mut shell, 30);
            save(&mut shell, &format!("{tag}-06b-memory-second"));
            missing.extend(shell.gfx.missing_icons().iter().cloned());
            if size != TextSize::Standard {
                continue;
            }
            // A change asked about first: Wi-Fi off, its card and its page.
            for _ in 0..3 {
                shell.key(Key::Back);
                settle(&mut shell, 40);
            }
            shell.key(Key::Ctrl('n'));
            settle(&mut shell, 20);
            let wifi = if crate::wifi::switched_on() { "turn wi-fi off" } else { "turn wi-fi on" };
            typed(&mut shell, wifi);
            shell.key(Key::Enter);
            settle(&mut shell, 120);
            save(&mut shell, &format!("{tag}-07-change-asks"));
            shell.key(Key::Enter);
            settle(&mut shell, 80);
            save(&mut shell, &format!("{tag}-08-change-confirm"));
            shell.key(Key::Back);
            settle(&mut shell, 140);
            // The morning's, asked for by name.
            shell.key(Key::Ctrl('n'));
            settle(&mut shell, 20);
            typed(&mut shell, "morning briefing");
            shell.key(Key::Enter);
            settle(&mut shell, 60);
            wait(&mut shell, 300);
            settle(&mut shell, 120);
            save(&mut shell, &format!("{tag}-09-briefing-morning"));
            missing.extend(shell.gfx.missing_icons().iter().cloned());
        }
    }
    // Offline (no key): the briefing from the template, with no model, and
    // what it remembers by the rules.
    crate::keys::Keys::default().save().ok();
    for look in ["catalyst", "bezel"] {
        let tag = format!("{look}-standard");
        let mut shell = fresh(look, TextSize::Standard);
        shell.show(Box::new(super::Assistant::new()));
        settle(&mut shell, 90);
        save(&mut shell, &format!("{tag}-10-offline-offered"));
        typed(&mut shell, "brief me");
        shell.key(Key::Enter);
        wait(&mut shell, 400);
        settle(&mut shell, 90);
        save(&mut shell, &format!("{tag}-11-offline-briefing"));
        shell.key(Key::Ctrl('n'));
        settle(&mut shell, 20);
        typed(&mut shell, "what do you remember?");
        shell.key(Key::Enter);
        settle(&mut shell, 90);
        save(&mut shell, &format!("{tag}-12-offline-memory"));
        missing.extend(shell.gfx.missing_icons().iter().cloned());
    }
    std::fs::remove_dir_all(&own).ok();
    if !missing.is_empty() {
        eprintln!("icons asked for that are not built in: {}", missing.into_iter().collect::<Vec<_>>().join(", "));
        std::process::exit(1);
    }
    println!("assistant shots in {}", dir.display());
}
