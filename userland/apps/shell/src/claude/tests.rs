//! Claude's pages with a made-up laptop and nothing on the network: the
//! session list, the transcript, the composer, the permission sheet, the
//! laptop out of reach, the keys; and the events from the bridge taken in.

use serde_json::json;

use super::link::{self, Row, Trouble};
use super::shots::{self, SESSION};
use super::ui::{ClaudeApp, Conversation, PairPage, Projects};
use crate::gfx::{Gfx, TextSize};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen, Settings};

struct Rig {
    robot: Robot,
    settings: Settings,
    say: Option<String>,
    g: Gfx,
}

impl Rig {
    fn new() -> Rig {
        Rig { robot: Robot::demo(), settings: Settings::default(), say: None, g: Gfx::new(bezel::Tokens::dark()) }
    }

    fn key(&mut self, s: &mut dyn Screen, key: Key) -> Nav {
        let mut cx = Cx { robot: &mut self.robot, settings: &mut self.settings, say: &mut self.say, at_rest: true };
        s.key(key, &mut cx)
    }

    fn draw(&mut self, s: &mut dyn Screen) {
        self.g.foot = crate::gfx::H;
        self.g.canvas.clear(self.g.tokens.ground);
        s.draw(&mut self.g, &self.robot);
        for _ in 0..60 {
            s.tick(1.0 / 60.0);
        }
        self.g.foot = crate::gfx::H;
        s.draw(&mut self.g, &self.robot);
    }

    fn typed(&mut self, s: &mut dyn Screen, words: &str) {
        for c in words.chars() {
            self.key(s, Key::Char(c));
        }
    }
}

fn pushed(nav: Nav) -> Option<String> {
    match nav {
        Nav::Push(s) => Some(s.title().to_owned()),
        _ => None,
    }
}

#[test]
fn the_list_shows_the_sessions_and_opens_one() {
    link::install_demo(shots::demo);
    let mut rig = Rig::new();
    let mut app = ClaudeApp::new();
    rig.draw(&mut app);
    // New session first; Down onto the first session, Enter opens it.
    assert_eq!(pushed(rig.key(&mut app, Key::Enter)).as_deref(), Some("New"));
    rig.key(&mut app, Key::Down);
    assert_eq!(pushed(rig.key(&mut app, Key::Enter)).as_deref(), Some("Claude"));
    let c = link::with(|s| s.convo.clone()).unwrap();
    assert_eq!((c.session.as_deref(), c.project.as_str(), c.loaded), (Some(SESSION), "CatalystLinux", false), "opened, its history asked for");
    // n: a new session from anywhere on the list; the tile says what waits.
    assert_eq!(pushed(rig.key(&mut app, Key::Char('n'))).as_deref(), Some("New"));
    assert_eq!(super::tile(), ("1 to answer".to_owned(), Some("WARN")));
    assert!(rig.g.missing_icons().is_empty(), "{:?}", rig.g.missing_icons());
}

#[test]
fn unpairing_takes_enter_twice() {
    link::install_demo(shots::demo);
    let mut rig = Rig::new();
    let mut app = ClaudeApp::new();
    rig.draw(&mut app);
    rig.key(&mut app, Key::End);
    rig.key(&mut app, Key::Enter);
    assert!(link::with(|s| s.paired.is_some()), "once: only armed");
    rig.key(&mut app, Key::Up);
    rig.key(&mut app, Key::Down);
    rig.key(&mut app, Key::Enter);
    assert!(link::with(|s| s.paired.is_some()), "moving off disarms it");
    rig.key(&mut app, Key::Enter);
    assert!(link::with(|s| s.paired.is_none() && s.sessions.is_none()), "twice: forgotten");
}

#[test]
fn unpaired_it_offers_the_two_ways_in() {
    link::install_demo(|_| {});
    let mut rig = Rig::new();
    let mut app = ClaudeApp::new();
    rig.draw(&mut app);
    assert_eq!(pushed(rig.key(&mut app, Key::Enter)).as_deref(), Some("Camera"));
    rig.key(&mut app, Key::Down);
    assert_eq!(pushed(rig.key(&mut app, Key::Enter)).as_deref(), Some("Pair"));
}

#[test]
fn the_transcript_opens_a_tools_row_on_enter() {
    link::install_demo(|s| {
        shots::demo(s);
        s.convo = Some(shots::convo());
    });
    let mut rig = Rig::new();
    let mut c = Conversation::new();
    rig.draw(&mut c);
    // Up three: from the newest (running tests) to the edit.
    for _ in 0..3 {
        rig.key(&mut c, Key::Up);
    }
    rig.key(&mut c, Key::Enter);
    let copied = c.copy(&rig.robot).unwrap();
    assert!(copied.starts_with("edited camera.rs +3 −1\n"), "{copied}");
    rig.draw(&mut c);
    // Opened, the row grows by its detail's lines; Enter again closes it.
    rig.key(&mut c, Key::Enter);
    rig.draw(&mut c);
    rig.key(&mut c, Key::End);
    assert!(c.copy(&rig.robot).unwrap().contains("cargo test"), "End: the newest again");
    assert!(rig.g.missing_icons().is_empty(), "{:?}", rig.g.missing_icons());
}

#[test]
fn the_composer_sends_and_waits_while_claude_works() {
    link::install_demo(|s| {
        shots::demo(s);
        s.working.clear();
        s.convo = Some(shots::convo());
    });
    let mut rig = Rig::new();
    let mut c = Conversation::new();
    assert!(c.wants_text() && c.dictation().is_some(), "it takes typing, and Alt dictates into it");
    rig.typed(&mut c, "run the tests");
    rig.key(&mut c, Key::Backspace);
    rig.typed(&mut c, "s");
    assert_eq!(c.copy(&rig.robot).as_deref(), Some("run the tests"));
    rig.key(&mut c, Key::Enter);
    let convo = link::with(|s| s.convo.clone()).unwrap();
    assert_eq!(convo.rows.last().map(|r| (r.kind.as_str(), r.text.as_str())), Some(("you", "run the tests")));
    assert_eq!(convo.unsent.as_deref(), Some("run the tests"), "kept until the laptop has it");
    assert!(link::with(|s| s.is_working(SESSION)), "held for its reply");
    // Busy: a second message waits, said so.
    rig.typed(&mut c, "and the build");
    rig.key(&mut c, Key::Enter);
    assert_eq!(rig.say.as_deref(), Some("Claude busy · ctrl x stops it"));
    assert_eq!(c.copy(&rig.robot).as_deref(), Some("and the build"), "still in the composer");
    // Dictation lands after what is typed.
    let mut cx = Cx { robot: &mut rig.robot, settings: &mut rig.settings, say: &mut rig.say, at_rest: true };
    c.dictated("too", &mut cx);
    assert_eq!(c.copy(&rig.robot).as_deref(), Some("and the build too"));
    // Tab: accept edits, then ask again; never anything else.
    rig.key(&mut c, Key::Tab);
    assert_eq!(link::with(|s| s.convo.as_ref().unwrap().mode.clone()), "edits");
    rig.key(&mut c, Key::Tab);
    assert_eq!(link::with(|s| s.convo.as_ref().unwrap().mode.clone()), "ask");
    rig.draw(&mut c);
}

#[test]
fn the_permission_sheet_asks_and_answers_once() {
    let now = link::laptop_now();
    link::install_demo(|s| {
        shots::demo(s);
        s.convo = Some(shots::convo());
        s.approvals.push(shots::approval(now));
    });
    let mut rig = Rig::new();
    let mut c = Conversation::new();
    rig.draw(&mut c);
    assert!(!c.wants_text(), "the sheet takes the keys, not the composer");
    assert!(c.key_sheet()[0].keys.iter().any(|(k, _)| *k == "y"));
    // A key in its first half second is let go: one meant for the composer.
    rig.key(&mut c, Key::Char('y'));
    assert!(link::with(|s| s.approval_in(SESSION).is_some()), "too soon");
    c.ambient_tick(0.6, &rig.robot);
    // Right chooses deny, Enter does it.
    rig.key(&mut c, Key::Right);
    rig.key(&mut c, Key::Enter);
    assert!(link::with(|s| s.approval_in(SESSION).is_none()), "answered");
    assert_eq!(rig.say.as_deref(), Some("denied"));
    rig.draw(&mut c);
    assert!(c.wants_text(), "the composer again");
    assert!(rig.g.missing_icons().is_empty(), "{:?}", rig.g.missing_icons());
}

#[test]
fn out_of_reach_it_says_so_and_keeps_what_it_had() {
    link::install_demo(|s| {
        shots::demo(s);
        s.trouble = Some(Trouble::Asleep);
    });
    let mut rig = Rig::new();
    let mut app = ClaudeApp::new();
    rig.draw(&mut app);
    rig.key(&mut app, Key::Down);
    assert_eq!(pushed(rig.key(&mut app, Key::Enter)).as_deref(), Some("Claude"), "the list kept still opens");
    // How curl and the funnel fail, in words.
    assert_eq!(link::curl_trouble(Some(6)), Trouble::Offline);
    for code in [7, 28, 35, 52, 56] {
        assert_eq!(link::curl_trouble(Some(code)), Trouble::Asleep, "{code}");
    }
    assert_eq!(link::status_trouble(502, ""), Trouble::Asleep, "funnel with no bridge behind it");
    assert_eq!(link::status_trouble(401, "not paired"), Trouble::NotPaired);
    assert_eq!(Trouble::Asleep.words(), "laptop asleep or unreachable");
    // Not paired on the laptop's side: Enter starts again.
    link::install_demo(|s| {
        shots::demo(s);
        s.trouble = Some(Trouble::NotPaired);
    });
    let mut app = ClaudeApp::new();
    rig.draw(&mut app);
    rig.key(&mut app, Key::Enter);
    assert!(link::with(|s| s.paired.is_none()));
}

#[test]
fn every_page_lists_its_keys_and_shows_them_along_its_foot() {
    link::install_demo(|s| {
        shots::demo(s);
        s.convo = Some(shots::convo());
    });
    let mut rig = Rig::new();
    let mut pages: Vec<(Box<dyn Screen>, &str)> = vec![
        (Box::new(ClaudeApp::new()), "n"),
        (Box::new(Projects::new()), "enter"),
        (Box::new(Conversation::new()), "ctrl x"),
        (Box::new(PairPage::by_code()), "enter"),
    ];
    for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
        rig.g.set_text_size(size);
        for (page, key) in pages.iter_mut() {
            let sheet = page.key_sheet();
            assert!(sheet.iter().flat_map(|g| g.keys.iter()).any(|(k, _)| k == key), "{}: {key}", page.title());
            rig.draw(page.as_mut());
            assert!(rig.g.foot < crate::gfx::H - 8.0, "{}: the keys along its foot", page.title());
        }
    }
    assert!(rig.g.missing_icons().is_empty(), "{:?}", rig.g.missing_icons());
}

#[test]
fn events_stream_into_the_open_conversation() {
    link::install_demo(|s| {
        shots::demo(s);
        s.working.clear();
        let mut c = shots::convo();
        c.since = ("e1".into(), 10);
        s.convo = Some(c);
    });
    let before = link::with(|s| s.convo.as_ref().unwrap().rows.len());
    // Older than its history: left out.
    link::apply(&json!({"ev": "row", "seq": 9, "epoch": "e1", "session": SESSION, "row": {"id": "old", "k": "claude", "text": "already there"}}));
    assert_eq!(link::with(|s| s.convo.as_ref().unwrap().rows.len()), before);
    link::apply(&json!({"ev": "turn", "seq": 11, "epoch": "e1", "session": SESSION, "state": "working"}));
    assert!(link::with(|s| s.is_working(SESSION)));
    link::apply(&json!({"ev": "row", "seq": 12, "epoch": "e1", "session": SESSION, "row": {"id": "m:s0", "k": "claude", "text": ""}}));
    link::apply(&json!({"ev": "append", "seq": 13, "epoch": "e1", "session": SESSION, "id": "m:s0", "text": "All 41 "}));
    link::apply(&json!({"ev": "append", "seq": 14, "epoch": "e1", "session": SESSION, "id": "m:s0", "text": "pass."}));
    let last = link::with(|s| s.convo.as_ref().unwrap().rows.last().cloned()).unwrap();
    assert_eq!(last.text, "All 41 pass.");
    // The whole message takes the streaming row's place.
    link::apply(&json!({"ev": "row", "seq": 15, "epoch": "e1", "session": SESSION, "row": {"id": "m:u:0", "k": "claude", "text": "All 41 pass.", "replaces": "m:s0"}}));
    let rows = link::with(|s| s.convo.as_ref().unwrap().rows.clone());
    assert_eq!(rows.len(), before + 1);
    assert_eq!(rows.last().unwrap().id, "m:u:0");
    // A request: kept, and said (the page is not open: no hold here).
    link::apply(&json!({"ev": "approval", "seq": 16, "epoch": "e1", "session": SESSION, "approval": {"id": "a9", "what": "run a command", "line": "cargo test", "where": "in CatalystLinux", "until": 0}}));
    assert_eq!(link::with(|s| s.approval_in(SESSION).map(|a| a.line.clone())).as_deref(), Some("cargo test"));
    link::apply(&json!({"ev": "approval_done", "seq": 17, "epoch": "e1", "session": SESSION, "id": "a9", "allowed": true}));
    assert!(link::with(|s| s.approval_in(SESSION).is_none()));
    // Done: the reply said, Claude no longer working.
    link::apply(&json!({"ev": "turn", "seq": 18, "epoch": "e1", "session": SESSION, "state": "done"}));
    assert!(!link::with(|s| s.is_working(SESSION)));
    let said = crate::notify::take_posted();
    assert!(said.iter().any(|n| n.title == "approval waiting" && n.urgent), "{said:?}");
    assert!(said.iter().any(|n| n.title == "reply ready" && n.body == "All 41 pass."), "{said:?}");
}

#[test]
fn a_row_already_there_is_not_shown_twice() {
    let mut c = shots::convo();
    let n = c.rows.len();
    let row = |id: &str, text: &str| Row { id: id.into(), kind: "claude".into(), text: text.into(), ..Row::default() };
    link::put_row_for_test(&mut c, row("other-id", "Now the tests for the camera."), None);
    assert_eq!(c.rows.len(), n, "the same words from the history");
    link::put_row_for_test(&mut c, row("new", "Something new."), None);
    assert_eq!(c.rows.len(), n + 1);
}

#[test]
fn a_laptops_code_is_read_and_kept_privately() {
    use catalyst_companion_proto::claude::{Invite, Key as PairKey};
    let invite = Invite { address: "https://laptop.tail1234.ts.net:10000".into(), key: PairKey::from_bytes([7; 32]), computer: "THOMAS-LAPTOP".into() };
    let text = invite.qr_text();
    assert!(super::is_pair_code(&text) && !super::is_pair_code("CATTEAM1 00 x"));
    assert_eq!(super::card(&text).1, "Claude · THOMAS-LAPTOP");
    assert_eq!(crate::camera::read_code(&text), crate::camera::Code::Text(text.clone()), "text, for the card above");
    let dir = std::env::temp_dir().join(format!("catalyst-claude-test-{}", std::process::id()));
    std::env::set_var("CATALYST_CLAUDE", &dir);
    let p = super::Paired { address: invite.address.clone(), key: invite.key.clone(), computer: invite.computer.clone(), since: 5 };
    super::save(&p).unwrap();
    assert_eq!(super::load(), Some(p));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join("pairing.json")).unwrap().permissions().mode() & 0o077, 0, "root's alone");
    }
    super::forget();
    assert_eq!(super::load(), None);
    std::fs::remove_dir_all(&dir).ok();
}
