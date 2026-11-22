//! The Assistant's tasks, briefings and memory, to look at
//! (`--assistant-shots <dir>`): what it offers to do, a task of several
//! steps under way and done, a briefing from its top and scrolled on, the
//! memory page, in both looks at each text size; at the usual size the
//! choices, a change asked about first, the morning's briefing, and each
//! look in light; then, offline, the briefing from the template, what it
//! remembers, requests done by rules and several timers asked about before
//! they stop.
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

/// How long a live activity's card stays open over the screen when it
/// starts (`liveactivity::OPEN_FOR`, 3.5 s), and a little: frames at 60 a
/// second.
const CARD_CLOSED: usize = 240;

fn settle(shell: &mut Shell, frames: usize) {
    for _ in 0..frames {
        if shell.tick(1.0 / 60.0) {
            shell.draw();
        }
    }
}

/// Frames at the handheld's pace, in real time: the threads the Assistant
/// waits on (the scripted service, the music search, a briefing's) keep
/// the same time as its own clock, as on the handheld.
fn live(shell: &mut Shell, ms: u64) {
    for _ in 0..ms * 60 / 1000 {
        std::thread::sleep(Duration::from_micros(16_667));
        settle(shell, 1);
    }
}

/// Live frames until the Assistant has nothing under way (or `ms`), then a
/// moment more for what came last to land.
fn until_done(shell: &mut Shell, ms: u64) {
    for _ in 0..ms / 50 {
        if !shell.top_busy() {
            break;
        }
        live(shell, 50);
    }
    settle(shell, 60);
}

fn typed(shell: &mut Shell, text: &str) {
    for c in text.chars() {
        shell.key(Key::Char(c));
    }
    settle(shell, 6);
}

/// Asked in a new conversation (once the island has said so), and sent.
fn ask(shell: &mut Shell, text: &str) {
    shell.key(Key::Ctrl('n'));
    settle(shell, 170);
    typed(shell, text);
    shell.key(Key::Enter);
}

fn fresh(look: &str, size: TextSize, light: bool) -> Shell {
    crate::music::forget_downloads_for_shots();
    std::fs::remove_file(crate::reminders::path()).ok();
    crate::reminders::with(|s| s.book = crate::reminders::Book::open(crate::reminders::path()));
    let memory = super::memory::path();
    std::fs::remove_file(&memory).ok();
    for fact in ["The robot is called Numbers", "The team is 5805", "Likes short answers, with the numbers first"] {
        super::memory::save(&memory, fact, "").ok();
    }
    std::fs::remove_dir_all(crate::config_dir().join("assistant-chats")).ok();
    while crate::clock::running_timers().first().is_some_and(|(id, ..)| crate::clock::remove_timer(*id)) {}
    let mut shell = Shell::new(Robot::demo());
    if look == "bezel" {
        let s = &mut shell.settings;
        (s.theme, s.wallpaper, s.accent) = (Theme::Bezel, crate::look::Wallpaper::Glow, bezel::Accent::Orange);
    }
    shell.settings.text = size;
    shell.settings.light = light;
    shell.apply_settings();
    settle(&mut shell, 100);
    shell
}

pub fn run(dir: &Path) {
    // A settings folder of this run's own: the keys, memory and reminders it writes.
    let own = std::env::temp_dir().join(format!("catalyst-assistant-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&own).ok();
    std::fs::create_dir_all(&own).ok();
    std::env::set_var("CATALYST_CONF", own.join("robot"));
    std::env::set_var("CATALYST_LOGS", own.join("logs"));
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let mut keys = crate::keys::Keys::default();
    keys.set("openai", "sk-proj-demo-7f3a");
    keys.save().ok();
    let save = |shell: &mut Shell, name: &str| {
        shell.draw();
        crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    };
    let mut missing = std::collections::BTreeSet::new();
    let mut sets: Vec<(&str, TextSize, bool)> = Vec::new();
    for look in ["catalyst", "bezel"] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            sets.push((look, size, false));
        }
        sets.push((look, TextSize::Standard, true));
    }
    for (look, size, light) in sets {
        let tag = format!("{look}-{}{}", size.name(), if light { "-light" } else { "" });
        let full = size == TextSize::Standard && !light;
        let mut shell = fresh(look, size, light);
        shell.show(Box::new(super::Assistant::new()));
        settle(&mut shell, 90);
        if !light {
            save(&mut shell, &format!("{tag}-00-start"));
        }
        // The task of two steps: asked, the download under way, then the
        // reminder done and the model thinking, then the answer.
        typed(&mut shell, EXAMPLE);
        shell.key(Key::Enter);
        // The service's turn (0.9 s), then the download found on its thread.
        live(&mut shell, 1300);
        if !light {
            save(&mut shell, &format!("{tag}-01-task-under-way"));
        }
        // Its thread done (real time), then the reminder a beat later,
        // and the service asked again.
        live(&mut shell, 900);
        if !light {
            save(&mut shell, &format!("{tag}-02-task-thinking"));
        }
        // The answer, once the download's card has gone back into the island.
        until_done(&mut shell, 6000);
        settle(&mut shell, CARD_CLOSED);
        save(&mut shell, &format!("{tag}-03-task-done"));
        // A briefing, the next match's (it is soon): from its top, then on.
        ask(&mut shell, "brief me");
        until_done(&mut shell, 8000);
        save(&mut shell, &format!("{tag}-04-briefing"));
        if !light {
            for _ in 0..3 {
                shell.key(Key::Down);
                settle(&mut shell, 12);
            }
            settle(&mut shell, 60);
            save(&mut shell, &format!("{tag}-04b-briefing-on"));
        }
        // What it remembers, from the choices.
        shell.key(Key::Tab);
        settle(&mut shell, 60);
        for _ in 0..7 {
            shell.key(Key::Down);
            settle(&mut shell, 4);
        }
        settle(&mut shell, 40);
        if full {
            save(&mut shell, &format!("{tag}-05-choices"));
        }
        shell.key(Key::Enter);
        settle(&mut shell, 70);
        save(&mut shell, &format!("{tag}-06-memory"));
        if !light {
            for _ in 0..3 {
                shell.key(Key::Down);
                settle(&mut shell, 8);
            }
            settle(&mut shell, 40);
            save(&mut shell, &format!("{tag}-06b-memory-forget-all"));
        }
        missing.extend(shell.gfx.missing_icons().iter().cloned());
        if !full {
            continue;
        }
        // A change asked about first: Wi-Fi off, its card and its page.
        for _ in 0..2 {
            shell.key(Key::Back);
            settle(&mut shell, 40);
        }
        let wifi = if crate::wifi::switched_on() { "turn wi-fi off" } else { "turn wi-fi on" };
        ask(&mut shell, wifi);
        live(&mut shell, 1500);
        save(&mut shell, &format!("{tag}-07-change-asks"));
        shell.key(Key::Enter);
        settle(&mut shell, 80);
        save(&mut shell, &format!("{tag}-08-change-confirm"));
        shell.key(Key::Back);
        settle(&mut shell, 140);
        // The morning's, asked for by name.
        ask(&mut shell, "morning briefing");
        until_done(&mut shell, 8000);
        save(&mut shell, &format!("{tag}-09-briefing-morning"));
        missing.extend(shell.gfx.missing_icons().iter().cloned());
    }
    // Offline (no key): the briefing from the template, what it remembers,
    // and requests done by rules, with no model.
    crate::keys::Keys::default().save().ok();
    for look in ["catalyst", "bezel"] {
        let tag = format!("{look}-standard");
        let mut shell = fresh(look, TextSize::Standard, false);
        shell.show(Box::new(super::Assistant::new()));
        settle(&mut shell, 90);
        save(&mut shell, &format!("{tag}-10-offline-offered"));
        typed(&mut shell, "brief me");
        shell.key(Key::Enter);
        until_done(&mut shell, 8000);
        save(&mut shell, &format!("{tag}-11-offline-briefing"));
        ask(&mut shell, "what do you remember?");
        until_done(&mut shell, 3000);
        save(&mut shell, &format!("{tag}-12-offline-memory"));
        ask(&mut shell, "start a 10 min timer and remind me to check the bumpers at queueing for Q42");
        until_done(&mut shell, 3000);
        settle(&mut shell, CARD_CLOSED);
        save(&mut shell, &format!("{tag}-13-offline-task"));
        ask(&mut shell, "set a 5 min timer, then stop the timers");
        live(&mut shell, 1000);
        settle(&mut shell, CARD_CLOSED);
        save(&mut shell, &format!("{tag}-14-offline-stop-asks"));
        shell.key(Key::Enter);
        settle(&mut shell, 80);
        save(&mut shell, &format!("{tag}-15-offline-stop-confirm"));
        shell.key(Key::Back);
        settle(&mut shell, 140);
        missing.extend(shell.gfx.missing_icons().iter().cloned());
    }
    std::fs::remove_dir_all(&own).ok();
    if !missing.is_empty() {
        eprintln!("icons asked for that are not built in: {}", missing.into_iter().collect::<Vec<_>>().join(", "));
        std::process::exit(1);
    }
    println!("assistant shots in {}", dir.display());
}
