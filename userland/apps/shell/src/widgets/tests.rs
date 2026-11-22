//! The widgets: their layout and its file lines, the packing, moving one,
//! every one fitting at every size, text size and look, the refresh policy,
//! and editing from Home's keys.

use super::edit::{insert, step, Dir};
use super::*;
use crate::gfx::TextSize;
use crate::look::Theme;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, HomeStyle, Screen, Shell};

fn w(kind: Kind, size: Size) -> Placed {
    Placed::new(kind, size, 0)
}

fn kinds(list: &[Placed]) -> Vec<Kind> {
    list.iter().map(|p| p.kind).collect()
}

/// `frames` of a sixtieth of a second, each drawn if anything moved (as
/// fullheight.rs's, which the device's own build has not).
fn settle(shell: &mut Shell, frames: usize) {
    for _ in 0..frames {
        if shell.tick(1.0 / 60.0) {
            shell.draw();
        }
    }
}

#[test]
fn a_layout_comes_back_from_its_lines_and_the_old_cards_become_widgets() {
    let layout = Layout { glance: vec![w(Kind::NextMatch, Size::Small), w(Kind::Photos, Size::Large)], home: vec![Placed::new(Kind::Clock, Size::Wide, 4), Placed::new(Kind::System, Size::Small, 9)] };
    let lines = layout.lines();
    assert_eq!(lines[0], ("widgets", "next_match:small,photos:large".to_owned()));
    assert_eq!(lines[1], ("home_widgets", "4:clock:wide,9:system:small".to_owned()));
    let glance = Layout::parse_glance(&lines[0].1);
    let home = Layout::parse_home(&lines[1].1);
    assert_eq!(kinds(&glance), [Kind::NextMatch, Kind::Photos]);
    assert_eq!(glance[1].size, Size::Large);
    assert_eq!(home.iter().map(|p| (p.kind, p.size, p.after)).collect::<Vec<_>>(), [(Kind::Clock, Size::Wide, 4), (Kind::System, Size::Small, 9)]);
    // Anything not understood is passed over.
    assert_eq!(kinds(&Layout::parse_glance("battery:small,nonsense,clock:huge,,music:wide")), [Kind::Battery, Kind::Music]);
    assert!(Layout::parse_home("x:clock:wide,3:clock").is_empty());
    // A new handheld: the next match over the battery and the robot; music
    // over reminders and the timers, a page further left.
    let start = Layout::starting();
    assert_eq!(
        start.glance.iter().map(|p| (p.kind, p.size)).collect::<Vec<_>>(),
        [(Kind::NextMatch, Size::Wide), (Kind::Battery, Size::Small), (Kind::Robot, Size::Small), (Kind::Music, Size::Wide), (Kind::Reminders, Size::Small), (Kind::Timers, Size::Small)]
    );
    let packed = Packed::of(0, &start.glance, 2);
    assert_eq!(packed.pages, 2, "two whole pages");
    assert_eq!(packed.cell(Slot::Widget(2)).map(|c| (c.page, c.row, c.col)), Some((0, 1, 1)), "the robot at the first page's foot, on the right");
    assert!(start.home.is_empty(), "Home's grid as it was");
    // The cards a handheld came with, never changed: the new start.
    assert_eq!(kinds(&Layout::from_cards("device,robot,next_match,last_match")), kinds(&start.glance));
    // Cards chosen: kept, in their order, each a wide widget.
    let chosen = Layout::from_cards("robot,note,pinned,robot,device");
    assert_eq!(kinds(&chosen), [Kind::Robot, Kind::Note, Kind::Pinned, Kind::System]);
    assert!(chosen.iter().all(|p| p.size == Size::Wide));
    // A value pinned in Tables goes on the glance page once.
    let mut l = Layout::starting();
    l.pin_value();
    l.pin_value();
    assert_eq!(l.glance.iter().filter(|p| p.kind == Kind::Pinned).count(), 1);
    // Every kind and size by its key, and sizes round.
    assert!(Kind::ALL.iter().all(|k| Kind::from_key(k.key()) == Some(*k)));
    assert_eq!(Size::Small.next().next().next(), Size::Small);
    assert_eq!((Size::Small.step(false), Size::Large.step(true), Size::Wide.step(true)), (Size::Small, Size::Large, Size::Large));
}

#[test]
fn packing_takes_the_first_place_that_fits_and_later_tiles_fill_the_holes() {
    // Three across: two small, then a wide one that will not fit beside them.
    let cells = pack(&[(1, 1), (1, 1), (2, 1), (1, 1), (1, 1)], 3);
    let at = |i: usize| (cells[i].page, cells[i].row, cells[i].col);
    assert_eq!([at(0), at(1)], [(0, 0, 0), (0, 0, 1)]);
    assert_eq!(at(2), (0, 1, 0), "the wide one on the next row");
    assert_eq!(at(3), (0, 0, 2), "the next small one fills the hole it left");
    assert_eq!(at(4), (0, 1, 2));
    // A large one takes two rows; on a page two across it is the whole page.
    let cells = pack(&[(1, 1), (2, 2), (1, 1)], 2);
    assert_eq!((cells[1].page, cells[1].row, cells[1].col, cells[1].w, cells[1].h), (1, 0, 0, 2, 2));
    assert_eq!((cells[2].page, cells[2].row, cells[2].col), (0, 0, 1), "back on the first page's free cell");
    // Nothing wider than the page.
    assert_eq!(pack(&[(2, 1)], 1)[0].w, 1);
    // Widgets before the app they were placed before; past the last, at the end.
    let list = [Placed::new(Kind::Clock, Size::Small, 2), Placed::new(Kind::Music, Size::Wide, 0), Placed::new(Kind::Photos, Size::Small, 99)];
    assert_eq!(sequence(3, &list), [Slot::Widget(1), Slot::App(0), Slot::App(1), Slot::Widget(0), Slot::App(2), Slot::Widget(2)]);
    let packed = Packed::of(3, &list, 3);
    assert_eq!(packed.pages, 2, "seven cells: the last on a page of its own");
    assert_eq!(packed.reading().first().map(|(s, _)| *s), Some(Slot::Widget(1)));
    // Up and down by column; down onto a short row lands on what is there.
    let packed = Packed::of(5, &[], 3);
    assert_eq!(packed.vertical(Slot::App(1), true), Some(Slot::App(4)));
    assert_eq!(packed.vertical(Slot::App(2), true), Some(Slot::App(4)), "a short last row");
    assert_eq!(packed.vertical(Slot::App(4), true), None);
    assert_eq!(packed.vertical(Slot::App(0), false), None);
    assert_eq!(packed.beside(Slot::App(2), 1), Some(Slot::App(3)));
}

#[test]
fn a_widget_moves_somewhere_new_with_each_arrow_and_stops_at_the_ends() {
    // The glance page: two small, a wide under them, a small on the next page.
    let board = vec![w(Kind::NextMatch, Size::Small), w(Kind::Battery, Size::Small), w(Kind::Music, Size::Wide), w(Kind::Clock, Size::Small)];
    let cell = |list: &[Placed], j: usize| {
        let c = Packed::of(0, list, 2).cell(Slot::Widget(j)).unwrap();
        (c.page, c.row, c.col)
    };
    // Right: the next match goes past the battery.
    let (list, k) = step(0, &board, 0, Dir::Right, 2).expect("room to the right");
    assert_eq!(list[k].kind, Kind::NextMatch);
    assert_eq!(cell(&list, k), (0, 0, 1));
    // Left from the first place: nowhere.
    assert!(step(0, &board, 0, Dir::Left, 2).is_none());
    assert!(step(0, &board, 0, Dir::Up, 2).is_none(), "the first page's top row");
    // Down: onto the row below, the wide one making room.
    let (list, k) = step(0, &board, 0, Dir::Down, 2).expect("a row below");
    assert_eq!(cell(&list, k).1, 1, "on the second row");
    // The wide one up: to the top row.
    let (list, k) = step(0, &board, 2, Dir::Up, 2).expect("a row above");
    assert_eq!(list[k].kind, Kind::Music);
    assert_eq!(cell(&list, k), (0, 0, 0));
    // The last on its own page: further right is nowhere.
    assert!(step(0, &board, 3, Dir::Right, 2).is_none());
    // Every widget stays on the board, each once.
    let (list, _) = step(0, &board, 1, Dir::Down, 2).unwrap();
    let mut got = kinds(&list);
    got.sort_by_key(|k| k.key());
    let mut want = kinds(&board);
    want.sort_by_key(|k| k.key());
    assert_eq!(got, want);
    // Among Home's apps: a widget moves past apps, its place among them counted afresh.
    let home = vec![Placed::new(Kind::Weather, Size::Small, 1)];
    let (moved, k) = step(10, &home, 0, Dir::Right, 3).unwrap();
    assert_eq!(moved[k].after, 2, "one app later");
    let (moved, k) = step(10, &home, 0, Dir::Down, 3).unwrap();
    assert_eq!(Packed::of(10, &moved, 3).cell(Slot::Widget(k)).map(|c| (c.row, c.col)), Some((1, 1)), "straight down, its column kept");
    // Inserted before a slot (the picker's widget, where the focus is).
    let (list, k) = insert(10, &home, Some(Slot::App(4)), w(Kind::Music, Size::Wide));
    assert_eq!((list[k].kind, list[k].after, list.len()), (Kind::Music, 4, 2));
    let (list, k) = insert(0, &board, None, w(Kind::Photos, Size::Large));
    assert_eq!(k, list.len() - 1, "at the end");
}

/// Every kind drawn at each size on a board `columns` across, in a look at
/// a text size: what it had to cut short.
fn cuts(theme: Theme, text: TextSize, columns: usize, light: bool, facts: &mut Facts, robot: &Robot) -> Vec<String> {
    let mut g = crate::gfx::Gfx::new(bezel::Tokens::of(bezel::Accent::Orange, if light { bezel::Tone::Light } else { bezel::Tone::Dark }));
    g.theme = theme;
    g.set_text_size(text);
    let grid = Grid::of(&g, columns);
    let mut out = Vec::new();
    for kind in Kind::ALL {
        for size in Size::ALL {
            let (w, h) = grid.size_of(size);
            facts.ready(kind, robot, true, (w as usize, h as usize));
            draw::CUTS.with(|c| c.borrow_mut().clear());
            let r = Rect::new(8.0, 28.0, w, h);
            for focused in [false, true] {
                let ctx = Ctx { facts, robot, pinned: Some("Loop/Robot/LastMs"), now: chrono::Local::now(), focused, radius: g.tile_radius(), laid: (w, h) };
                content(&mut g, r, kind, size, &ctx);
            }
            let cut: Vec<String> = draw::CUTS.with(|c| c.borrow().clone());
            out.extend(cut.into_iter().map(|s| format!("{kind:?} {size:?} at {} ({columns} across, {theme:?}): {s:?}", text.name())));
            assert!(g.missing_icons().is_empty(), "{kind:?} asks for {:?}", g.missing_icons());
        }
    }
    out
}

#[test]
fn every_widget_fits_whole_at_every_size_text_size_and_look() {
    crate::shell::tests::sandbox();
    let robot = Robot::demo();
    crate::nowplaying::set(Some(crate::nowplaying::NowPlaying {
        title: "Around the World".into(),
        artist: "Daft Punk".into(),
        album: "Homework".into(),
        cover: None,
        position: 83.0,
        duration: 427.0,
        playing: true,
        at: std::time::Instant::now(),
    }));
    let mut facts = Facts::default();
    // Reminders and a note to show (set by hand: the tests' book is their own).
    facts.reminders = vec![
        crate::reminders::Upcoming { id: 1, text: "charge the spares".into(), when: "in 25 min".into(), at: Some(0), due: false, pit: true, note: None },
        crate::reminders::Upcoming { id: 2, text: "pack the bumpers".into(), when: "15:30".into(), at: Some(0), due: false, pit: true, note: None },
    ];
    facts.hold(Kind::Reminders);
    facts.note = Some(super::facts::NoteFact { title: "Pit list for Q31".into(), lines: vec!["swap battery 3".into(), "zip-tie the intake's cable".into()], file: std::env::temp_dir().join("x.txt"), pinned: true });
    facts.hold(Kind::Note);
    let mut wrong = Vec::new();
    for theme in [Theme::Catalyst, Theme::Bezel] {
        for text in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            // Home's grid three across (two at the large size), and the glance page.
            let boards: &[usize] = if text == TextSize::Large { &[2] } else { &[3, 2] };
            for &columns in boards {
                wrong.extend(cuts(theme, text, columns, false, &mut facts, &robot));
            }
        }
    }
    wrong.extend(cuts(Theme::Catalyst, TextSize::Standard, 3, true, &mut facts, &robot));
    // And with nothing to show: each says why, whole.
    crate::nowplaying::set(None);
    let mut empty = Facts::default();
    empty.hold(Kind::Reminders);
    empty.hold(Kind::Note);
    for text in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
        wrong.extend(cuts(Theme::Bezel, text, if text == TextSize::Large { 2 } else { 3 }, false, &mut empty, &Robot::none()));
    }
    assert!(wrong.is_empty(), "{} cut short:\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn what_is_read_is_read_again_only_when_due() {
    crate::shell::tests::sandbox();
    let robot = Robot::demo();
    let mut facts = Facts::default();
    // Read on a timestamp, on the reading thread: nothing to show until it
    // answers (and a frame drawn soon to look for the answer), then kept.
    assert!(!facts.known(Kind::System));
    facts.prepare(Kind::System, &robot, true, (0, 0), false);
    assert!(facts.due(Kind::System).is_some_and(|at| at <= std::time::Instant::now() + std::time::Duration::from_millis(200)), "looked for soon");
    facts.ready(Kind::System, &robot, true, (0, 0));
    assert!(facts.known(Kind::System) && facts.system.memory.is_some(), "the handheld's figures");
    facts.system = super::facts::System::default();
    facts.prepare(Kind::System, &robot, true, (0, 0), false);
    std::thread::sleep(std::time::Duration::from_millis(50));
    facts.prepare(Kind::System, &robot, true, (0, 0), false);
    assert!(facts.system.memory.is_none(), "not read again within its period");
    let due = facts.due(Kind::System).expect("a time to read it again");
    let left = due.saturating_duration_since(std::time::Instant::now()).as_secs_f32();
    assert!((4.0..=5.0).contains(&left), "{left} s");
    // Asked to (Enter on it, or back from an app): at once.
    facts.changed(Kind::System);
    facts.ready(Kind::System, &robot, true, (0, 0));
    assert!(facts.system.memory.is_some());
    // The battery is the shell's own reading: never read here.
    facts.prepare(Kind::Battery, &robot, true, (0, 0), false);
    assert!(facts.battery.is_none());
    // On events: the schedule's generation; what is in memory has no time of its own.
    facts.prepare(Kind::NextMatch, &robot, true, (0, 0), false);
    assert!(facts.view.is_some());
    assert!(facts.due(Kind::NextMatch).is_none() && facts.due(Kind::Robot).is_none() && facts.due(Kind::Music).is_none());
    // What an app may change is read afresh once Home is back in front.
    facts.ready(Kind::Note, &robot, true, (0, 0));
    assert!(facts.due(Kind::Note).is_some());
    facts.invalidate();
    assert!(facts.due(Kind::Note).is_none(), "due now");
}

/// A shell on the demo robot, settled on Home, with these on the glance
/// page and among the apps.
fn shell_with(glance: Vec<Placed>, home: Vec<Placed>) -> Shell {
    crate::shell::tests::sandbox();
    let mut shell = Shell::new(Robot::demo());
    shell.settings.widgets = Layout { glance, home };
    shell.apply_settings();
    settle(&mut shell, 120);
    shell
}

fn title(shell: &Shell) -> String {
    shell.top_title().to_owned()
}

#[test]
fn e_edits_the_glance_page_and_every_change_is_kept() {
    // The next match and the battery side by side, music under them.
    let mut shell = shell_with(vec![w(Kind::NextMatch, Size::Small), w(Kind::Battery, Size::Small), w(Kind::Music, Size::Wide)], Vec::new());
    shell.key(Key::Left);
    settle(&mut shell, 60);
    assert_eq!(title(&shell), "Glance");
    // e: editing, the widget in focus (the battery, nearest Home) chosen.
    shell.key(Key::Char('e'));
    assert_eq!(title(&shell), "Widgets");
    // Left: it changes places with the next match.
    shell.key(Key::Left);
    assert_eq!(kinds(&shell.settings.widgets.glance), [Kind::Battery, Kind::NextMatch, Kind::Music]);
    // r: wide; the next match now on a row of its own.
    shell.key(Key::Char('r'));
    assert_eq!(shell.settings.widgets.glance[0].size, Size::Wide);
    // d once asks; any other key keeps it; d twice takes it off.
    shell.key(Key::Char('d'));
    assert_eq!(shell.settings.widgets.glance.len(), 3);
    shell.key(Key::Char('k'));
    shell.key(Key::Char('d'));
    shell.key(Key::Char('d'));
    assert_eq!(kinds(&shell.settings.widgets.glance), [Kind::NextMatch, Kind::Music]);
    // a: the picker, in place; Right turns it; Enter keeps it.
    shell.key(Key::Char('a'));
    assert_eq!(title(&shell), "Add");
    assert_eq!(shell.settings.widgets.glance.len(), 2, "not kept until chosen");
    shell.key(Key::Right);
    shell.key(Key::Up);
    shell.key(Key::Enter);
    let glance = shell.settings.widgets.glance.clone();
    assert_eq!(glance.len(), 3);
    assert!(!matches!(glance[1].kind, Kind::NextMatch | Kind::Music), "a kind not on the board yet: {:?}", kinds(&glance));
    // Esc in the picker goes back, the board as it was.
    shell.key(Key::Char('a'));
    shell.key(Key::Back);
    assert_eq!(shell.settings.widgets.glance.len(), 3);
    assert_eq!(title(&shell), "Widgets");
    // Enter: done. It was all kept in the settings file as it was made.
    settle(&mut shell, 60);
    shell.key(Key::Enter);
    assert_eq!(title(&shell), "Glance");
    let saved = std::fs::read_to_string(crate::config_dir().join("settings")).unwrap_or_default();
    let line = saved.lines().find(|l| l.starts_with("widgets=")).expect("the widgets' line");
    assert_eq!(Layout::parse_glance(line.trim_start_matches("widgets=")).len(), 3);
}

#[test]
fn widgets_sit_among_the_apps_and_enter_acts_on_them() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static PAUSED: AtomicBool = AtomicBool::new(false);
    // Music after the first app, the clock after the third.
    let mut shell = shell_with(Vec::new(), vec![Placed::new(Kind::Music, Size::Wide, 1), Placed::new(Kind::Clock, Size::Small, 3)]);
    crate::nowplaying::set(Some(crate::nowplaying::NowPlaying { title: "Da Funk".into(), artist: "Daft Punk".into(), album: String::new(), cover: None, position: 1.0, duration: 300.0, playing: true, at: std::time::Instant::now() }));
    crate::nowplaying::on_control(|c| PAUSED.store(c == crate::nowplaying::Control::PlayPause, Ordering::Relaxed));
    // Right from the first app: the music widget, then the next app.
    shell.key(Key::Right);
    settle(&mut shell, 30);
    let spot = shell.stack_spot().expect("the widget has the focus");
    assert!(spot.w > 150.0, "two tiles wide: {spot:?}");
    // Enter on it plays or pauses, in place.
    shell.key(Key::Enter);
    settle(&mut shell, 30);
    assert!(PAUSED.load(Ordering::Relaxed), "asked the player");
    assert_eq!(shell.stack_depth(), 1, "nothing opened");
    // e on Home's grid edits the grid; Right moves the widget one app on.
    shell.key(Key::Char('e'));
    assert_eq!(title(&shell), "Widgets");
    shell.key(Key::Right);
    assert_eq!(shell.settings.widgets.home[0].after, 2);
    shell.key(Key::Back);
    assert_eq!(title(&shell), "Catalyst");
    // Carousel: no widgets among the apps; e edits the glance page instead.
    shell.settings.home = HomeStyle::Carousel;
    shell.apply_settings();
    settle(&mut shell, 60);
    shell.key(Key::Char('e'));
    settle(&mut shell, 30);
    assert_eq!(title(&shell), "Add", "the glance page is empty: the picker opens");
}

#[test]
fn settings_sends_home_into_edit_mode() {
    let mut shell = shell_with(Layout::starting().glance, Vec::new());
    let (index, ..) = crate::screens::setting_sections().into_iter().find(|(_, n, _)| *n == "Widgets").expect("in search");
    shell.show(crate::screens::open_setting(index));
    settle(&mut shell, 60);
    assert_eq!(title(&shell), "Widgets");
    shell.key(Key::Enter);
    settle(&mut shell, 60);
    assert_eq!(shell.stack_depth(), 1, "back on Home");
    assert_eq!(title(&shell), "Widgets", "editing the glance page");
    shell.key(Key::Enter);
    assert_eq!(title(&shell), "Glance");
}

#[test]
fn home_asks_for_a_frame_only_when_a_widget_changes_by_itself() {
    crate::shell::tests::sandbox();
    let robot = Robot::demo();
    let mut g = crate::gfx::Gfx::new(bezel::Tokens::dark());
    let mut home = crate::screens::Home::new();
    // A clock on the glance page, in sight.
    g.widgets = Layout { glance: vec![w(Kind::Clock, Size::Small)], home: Vec::new() };
    let (mut settings, mut say) = (crate::shell::Settings::default(), None);
    settings.widgets = g.widgets.clone();
    let mut robot_mut = Robot::demo();
    home.key(Key::Left, &mut Cx { robot: &mut robot_mut, settings: &mut settings, say: &mut say, at_rest: true });
    // The page turns in and the widget in focus lifts; then all is still.
    for _ in 0..180 {
        home.tick(1.0 / 60.0);
        home.draw(&mut g, &robot);
    }
    // At rest: nothing to draw until the minute turns.
    assert!(!home.tick(0.0), "nothing moves at rest");
    let next = home.next_change_for_tests().expect("the minute's turn");
    let left = next.saturating_duration_since(std::time::Instant::now()).as_secs_f32();
    assert!(left <= 60.1, "within the minute: {left}");
    // Once it is due: one frame, and no more until Home has drawn.
    home.pretend_due();
    assert!(home.tick(0.0), "the minute turned");
    assert!(!home.tick(0.0), "asked once");
    assert!(!home.tick(0.0));
    home.draw(&mut g, &robot);
    assert!(!home.tick(0.0), "drawn: the next minute is a minute away");
    // Nothing in sight (the apps in view, no widgets among them): nothing asked.
    let mut home = crate::screens::Home::new();
    g.widgets = Layout { glance: vec![w(Kind::Clock, Size::Small)], home: Vec::new() };
    home.draw(&mut g, &robot);
    assert!(home.next_change_for_tests().is_none(), "the glance page is out of sight");
}
