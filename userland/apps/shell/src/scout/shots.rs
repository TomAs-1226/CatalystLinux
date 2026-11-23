//! `catalyst-shell --scout-shots <dir>`: Scout's pages from a demo event
//! (the demo robot's, Q1 to Q26 scouted from all six places), dark and
//! light at each text size, as `<dir>/<page>-<dark|light>-<size>.png`.

use std::path::Path;

use super::practice::{Session, Split};
use super::stats::Picklist;
use super::store::{Record, Station, Store, Version};
use crate::fullheight::settle;
use crate::gfx::TextSize;
use crate::platform::Key;
use crate::shell::{Screen, Shell};

const SCOUTS: [&str; 6] = ["maya", "sam", "ana", "ben", "li", "jo"];

/// A team's knack for the game, 0.3 to 1, the same every time.
fn knack(team: u32) -> f32 {
    let h = team.wrapping_mul(2_654_435_761) >> 8;
    0.3 + (h % 700) as f32 / 1000.0
}

/// A little give, match to match: −1 to 1.
fn wobble(team: u32, n: u32, k: u32) -> f32 {
    let h = (team.wrapping_mul(31).wrapping_add(n.wrapping_mul(977)).wrapping_add(k.wrapping_mul(7919))).wrapping_mul(2_246_822_519) >> 9;
    (h % 2001) as f32 / 1000.0 - 1.0
}

const NOTES: [&str; 6] = [
    "Quick ground intake, shoots from the trench; lost a bumper in the last 20 s.",
    "Played defence all of shift 3, pinned the blue shooter twice.",
    "Climbed to level 3 in 6 s. Missed a lot from far out.",
    "Stopped for 15 s in teleop, came back. Watch their battery.",
    "Fed fuel to the hub from the outpost; great passer, few shots of its own.",
    "Very fast across the bump; drove cleanly round traffic.",
];

/// The demo event's records: Q1 to Q26, every place, with notes now and then.
fn seed(root: &Path) {
    let event = super::event::demo();
    let mut store = Store::load(root);
    let now = super::now();
    for m in event.matches.iter().take(26) {
        let n: u32 = m.label[1..].parse().unwrap_or(0);
        for s in Station::ALL {
            let Some(team) = s.team_in(m) else { continue };
            let k = knack(team);
            let w = |i: u32| wobble(team, n, i);
            let level = ((k - 0.3) * 5.5 + w(5) * 0.8).clamp(0.0, 3.0).round();
            let mut values = vec![
                ("auto.fuel", (k * 11.0 + w(1) * 3.0).max(0.0).round()),
                ("auto.missed", ((1.0 - k) * 5.0 + w(2) * 2.0).max(0.0).round()),
                ("auto.tower", if k + w(3) * 0.2 > 0.75 { 1.0 } else { 0.0 }),
                ("teleop.fuel", (k * 46.0 + w(4) * 9.0).max(0.0).round()),
                ("teleop.missed", ((1.0 - k) * 14.0 + w(6) * 4.0).max(0.0).round()),
                ("teleop.passed", ((0.8 - k).max(0.0) * 30.0 + w(7) * 4.0).max(0.0).round()),
                ("endgame.tower", level),
                ("match.minor", if w(8) > 0.6 { 1.0 } else { 0.0 }),
                ("match.major", if w(9) > 0.9 { 1.0 } else { 0.0 }),
                ("after.driving", (1.0 + k * 4.0 + w(10) * 0.7).clamp(1.0, 5.0).round()),
            ];
            if w(11) > 0.3 {
                values.push(("after.defense", (2.0 + w(12) * 2.0).clamp(1.0, 5.0).round()));
            }
            let scout = SCOUTS[s.0 as usize];
            let at = now - (31 - n as i64) * 413 + 200;
            let mut r = Record {
                event: event.key.clone(),
                match_key: m.key.clone(),
                label: m.label.clone(),
                team,
                station: s,
                form: "rebuilt-2026".into(),
                values: values.into_iter().filter(|(_, v)| *v != 0.0).map(|(c, v)| (c.to_owned(), v)).collect(),
                scout: scout.into(),
                ..Record::default()
            };
            if (n + s.0 as u32) % 4 == 0 {
                r.texts.insert("after.notes".into(), NOTES[((n + team) % 6) as usize].into());
            }
            r.stamp(at, scout, "scouted".into());
            store.put(r).ok();
        }
    }
    // A note corrected later, by another scout.
    if let Some(mut r) = store.get("2026nyro_qm14", 5805).cloned() {
        r.history.push(Version { at: r.updated + 900, by: "sam".into(), change: "teleop fuel in 30 → 32".into() });
        store.put(r).ok();
    }
    // The picklist, part ordered by hand: three teams moved up, two not to pick.
    let mut teams: Vec<u32> = store.records.iter().map(|r| r.team).collect();
    teams.sort_unstable();
    teams.dedup();
    teams.sort_by(|a, b| knack(*b).total_cmp(&knack(*a)));
    let mut list = Picklist { event: event.key.clone(), picks: Vec::new() };
    list.add_missing(&teams);
    list.to_top(5);
    list.shift(3, -2);
    let dnp: Vec<usize> = [7usize, 12].iter().filter(|&&i| i < list.picks.len()).copied().collect();
    for i in dnp.into_iter().rev() {
        list.toggle_dnp(i);
    }
    list.picks[0].note = "first pick: level 3 every match".into();
    // What the scouts wrote of it, in two of its matches.
    let first = list.picks[0].team;
    let mine: Vec<Record> = store.records.iter().filter(|r| r.team == first).take(3).cloned().collect();
    for (k, mut r) in mine.into_iter().enumerate().skip(1) {
        r.texts.insert("after.notes".into(), NOTES[(k * 2) % NOTES.len()].into());
        store.put(r).ok();
    }
    list.picks[2].note = "defends well".into();
    if let Some(p) = list.picks.iter_mut().find(|p| p.dnp) {
        p.note = "tipped twice".into();
    }
    list.save(root).ok();
    // Two practice sessions.
    let split = |d: &str, s: f32| Split { driver: d.into(), secs: s };
    for (k, splits) in [
        vec![split("maya", 11.2), split("maya", 10.4), split("sam", 12.1), split("sam", 9.8), split("maya", 9.6), split("sam", 10.9), split("maya", 9.1)],
        vec![split("sam", 13.0), split("sam", 11.7), split("maya", 10.9), split("maya", 10.2), split("sam", 11.1)],
    ]
    .into_iter()
    .enumerate()
    {
        super::practice::keep(&Session { started: now - 86_400 * (k as i64 + 1) + 3600 * 3, splits }).ok();
    }
    std::fs::write(root.join("practice").join("drivers"), "maya\nsam").ok();
    super::Me { name: "maya".into(), station: Station(1) }.save();
}

fn press(shell: &mut Shell, keys: &[Key]) {
    for key in keys {
        shell.key(*key);
        settle(shell, 30);
    }
    settle(shell, 60);
}

fn typed(shell: &mut Shell, text: &str) {
    for c in text.chars() {
        shell.key(Key::Char(c));
        settle(shell, 2);
    }
    settle(shell, 40);
}

fn event_key() -> String {
    super::event::demo().key
}

/// The picklist's first team: noted on the list and by the scouts.
fn first_pick() -> u32 {
    Picklist::load(&super::dir(), &event_key()).picks.first().map_or(254, |p| p.team)
}

pub fn gallery(dir: &Path) {
    std::fs::create_dir_all(dir).expect("the shots folder");
    let state = std::env::temp_dir().join(format!("catalyst-scout-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&state).ok();
    std::fs::create_dir_all(&state).expect("a state folder");
    std::env::set_var("CATALYST_CONF", state.join("robot"));
    std::env::set_var("CATALYST_SCOUT", state.join("scout"));
    let q27 = || {
        let e = super::event::demo();
        let m = e.matches.iter().find(|m| m.label == "Q27").cloned().expect("Q27");
        let team = Station(1).team_in(&m).expect("a team at red 2");
        super::scouting::Scouting::new(&e.key, &m.key, &m.label, team, Station(1))
    };
    let stops: Vec<(&str, Box<dyn Fn() -> Box<dyn Screen>>, Vec<Key>, &str)> = vec![
        ("01-home", Box::new(|| Box::new(super::Scout::new())), vec![], ""),
        ("02-home-rows", Box::new(|| Box::new(super::Scout::new())), vec![Key::Right, Key::Down], ""),
        ("03-matches", Box::new(|| Box::new(super::home::Matches::new(Some(super::event::demo()), Station(1)))), vec![], ""),
        ("04-by-hand", Box::new(|| Box::new(super::home::ByHand::new(Station(4)))), vec![], "42"),
        ("05-auto-ready", Box::new(move || Box::new(q27())), vec![], ""),
        (
            "06-auto",
            Box::new(move || {
                let mut s = q27();
                s.demo(&[("auto.fuel", 5.0), ("auto.missed", 2.0), ("auto.tower", 1.0)], 11.0, None);
                Box::new(s)
            }),
            vec![],
            "",
        ),
        (
            "07-teleop",
            Box::new(move || {
                let mut s = q27();
                s.demo(&[("auto.fuel", 6.0), ("auto.missed", 2.0), ("auto.tower", 1.0), ("teleop.fuel", 23.0), ("teleop.missed", 7.0), ("teleop.passed", 4.0), ("match.minor", 1.0)], 74.0, None);
                Box::new(s)
            }),
            vec![],
            "",
        ),
        (
            "08-endgame",
            Box::new(move || {
                let mut s = q27();
                s.demo(&[("auto.fuel", 6.0), ("auto.missed", 2.0), ("auto.tower", 1.0), ("teleop.fuel", 38.0), ("teleop.missed", 9.0), ("teleop.passed", 6.0), ("endgame.tower", 2.0), ("match.minor", 1.0)], 141.0, None);
                Box::new(s)
            }),
            vec![],
            "",
        ),
        (
            "09-after",
            Box::new(move || {
                let mut s = q27();
                s.demo(&[("auto.fuel", 6.0), ("auto.missed", 2.0), ("auto.tower", 1.0), ("teleop.fuel", 41.0), ("teleop.missed", 9.0), ("teleop.passed", 6.0), ("endgame.tower", 3.0), ("match.minor", 1.0), ("after.driving", 4.0)], 160.0, None);
                s.notes_text("Fast cycles from the depot; climbed to level 3 with 4 s left.");
                Box::new(s)
            }),
            vec![Key::Down, Key::Down],
            "",
        ),
        (
            "10-notes",
            Box::new(move || {
                let mut s = q27();
                s.demo(&[("teleop.fuel", 41.0), ("after.driving", 4.0)], 160.0, None);
                Box::new(s)
            }),
            vec![Key::Down, Key::Down, Key::Down, Key::Enter],
            "fast cycles from the depot",
        ),
        ("11-teams", Box::new(|| Box::new(super::teams::Teams::new(event_key()))), vec![Key::Down], ""),
        ("12-teams-by-teleop", Box::new(|| Box::new(super::teams::Teams::new(event_key()))), vec![Key::Right, Key::Right], ""),
        ("13-team", Box::new(|| Box::new(super::teams::TeamPage::new(event_key(), first_pick()))), vec![], ""),
        ("14-team-notes", Box::new(|| Box::new(super::teams::TeamPage::new(event_key(), first_pick()))), vec![Key::End, Key::Up], ""),
        ("15-picklist", Box::new(|| Box::new(super::teams::PicklistScreen::new(event_key()))), vec![Key::Down], ""),
        ("16-picklist-foot", Box::new(|| Box::new(super::teams::PicklistScreen::new(event_key()))), vec![Key::End], ""),
        ("17-share", Box::new(|| Box::new(super::sharing::ShareMenu::new(event_key(), None))), vec![], ""),
        (
            "18-codes",
            Box::new(|| {
                let form = super::form();
                let event = event_key();
                let codes = super::with_store(|s| {
                    let all: Vec<&Record> = s.of_event(&event).collect();
                    super::share::chunks(&form, &event, &all)
                });
                Box::new(super::sharing::Codes::new("Event".into(), codes))
            }),
            vec![Key::Char(' '), Key::Right, Key::Right],
            "",
        ),
        (
            "19-practice",
            Box::new(|| {
                let mut p = super::practice::Practice::new();
                let split = |d: &str, s: f32| Split { driver: d.into(), secs: s };
                p.demo(vec![split("maya", 11.4), split("maya", 10.2), split("sam", 12.6), split("sam", 10.8), split("maya", 9.7), split("maya", 9.3)], 6.4);
                Box::new(p)
            }),
            vec![],
            "",
        ),
        ("20-sessions", Box::new(|| Box::new(super::practice::Sessions::new())), vec![], ""),
        ("21-session", Box::new(|| Box::new(super::practice::Sessions::new())), vec![Key::Enter], ""),
    ];
    let sizes = [TextSize::Compact, TextSize::Standard, TextSize::Large];
    let mut count = 0;
    for light in [false, true] {
        for size in sizes {
            for (name, make, keys, text) in &stops {
                // Each page from the same records, whatever the one before kept.
                std::fs::remove_dir_all(state.join("scout")).ok();
                seed(&state.join("scout"));
                super::reload();
                let mut shell = crate::fullheight::shell("catalyst", size);
                shell.settings.screen_off = 0;
                shell.settings.light = light;
                shell.apply_settings();
                settle(&mut shell, 30);
                shell.show(make());
                settle(&mut shell, 90);
                press(&mut shell, keys);
                if !text.is_empty() {
                    typed(&mut shell, text);
                }
                shell.draw();
                let file = dir.join(format!("{name}-{}-{}.png", if light { "light" } else { "dark" }, size.name()));
                crate::platform::sim::save_png(&shell.gfx.canvas, &file, 2);
                count += 1;
            }
        }
    }
    println!("{count} shots in {}", dir.display());
    std::fs::remove_dir_all(&state).ok();
}
