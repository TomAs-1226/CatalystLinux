//! The vault as a whole: made, opened, locked, moved to a new passcode, fed
//! an export; its pages drawn whole; and its secrets kept out of the
//! island, the logs, the clipboard's history, Spotlight and the card.

use std::time::Duration;

use super::screen::EntryPage;
use super::*;
use crate::gfx::{Gfx, TextSize, H};
use crate::platform::Key;
use crate::shell::{Cx, Nav, Screen, Settings};

/// Demo secrets: every one is looked for where it must never be.
const PASSWORD: &str = "Tr0ub4dor&3-demo";
const USERNAME: &str = "pit-crew@example.org";
const NOTE: &str = "locker combination 12-34-56";
const TOTP_SECRET: &str = "JBSWY3DPEHPK3PXP";

/// A passcode set in this test's own folder, quick to hash.
fn with_passcode(code: &str) -> String {
    crate::passcode::test_store();
    let (phc, _, _) = crate::passcode::make(code).unwrap();
    crate::passcode::keep(&phc).unwrap();
    phc
}

fn wait<T>(rx: std::sync::mpsc::Receiver<T>) -> T {
    rx.recv_timeout(Duration::from_secs(30)).expect("the vault's thread answered")
}

fn opened(outcome: Outcome) {
    match outcome {
        Outcome::Opened(u) => u.install(),
        Outcome::Wrong => panic!("wrong passcode"),
        Outcome::Failed(why) => panic!("failed: {why}"),
    }
}

fn demo_login() -> Entry {
    let mut e = Entry::new(Kind::Login, entry::new_id(), 1_763_700_000);
    e.set(Field::Title, "GitHub");
    e.set(Field::Username, USERNAME);
    e.set(Field::Password, PASSWORD);
    e.set(Field::Url, "https://github.com");
    e.set(Field::Notes, NOTE);
    e.set(Field::Totp, &format!("otpauth://totp/GitHub:pit?secret={TOTP_SECRET}&issuer=GitHub"));
    e
}

/// A vault made under "2468" with one demo login, open.
fn open_vault() -> String {
    let phc = with_passcode("2468");
    opened(wait(begin_create(Zeroizing::new("2468".into()))));
    change(|entries| entries.push(demo_login())).unwrap();
    flush();
    phc
}

#[test]
fn a_vault_is_made_with_the_passcode_and_opens_with_it_alone() {
    let _ = open_vault();
    assert!(is_open() && exists());
    lock();
    assert!(!is_open());
    // A wrong passcode, then the right one.
    assert!(matches!(wait(begin_unlock(Zeroizing::new("2469".into()))), Outcome::Wrong));
    assert!(!is_open());
    opened(wait(begin_unlock(Zeroizing::new("2468".into()))));
    assert_eq!(with(|e| e.len()), Some(1));
    assert_eq!(get(with(|e| e[0].id).unwrap()).unwrap().password, PASSWORD);
    // It can only be made with the lock screen's own passcode.
    delete().unwrap();
    assert!(matches!(wait(begin_create(Zeroizing::new("1357".into()))), Outcome::Wrong));
    assert!(!exists());
}

#[test]
fn nothing_secret_is_ever_on_the_card_in_the_clear() {
    let _ = open_vault();
    let file = std::fs::read(path()).unwrap();
    for secret in [PASSWORD, USERNAME, NOTE, TOTP_SECRET, "GitHub"] {
        assert!(!file.windows(secret.len()).any(|w| w == secret.as_bytes()), "{secret} on the card");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(path()).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // No file of it left half-written beside it.
    let dir = path().parent().unwrap().to_path_buf();
    assert!(std::fs::read_dir(dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().contains("saving")));
}

#[test]
fn a_changed_byte_on_the_card_does_not_open() {
    let _ = open_vault();
    lock();
    let good = std::fs::read(path()).unwrap();
    let mut bad = good.clone();
    let at = bad.len() - 40;
    bad[at] ^= 0x20;
    std::fs::write(path(), &bad).unwrap();
    match wait(begin_unlock(Zeroizing::new("2468".into()))) {
        Outcome::Failed(why) => assert!(why.contains("changed or damaged"), "{why}"),
        _ => panic!("a changed vault opened"),
    }
    assert!(!is_open());
    std::fs::write(path(), &good).unwrap();
    opened(wait(begin_unlock(Zeroizing::new("2468".into()))));
}

#[test]
fn changing_the_passcode_wraps_the_vault_again_and_a_failed_change_leaves_it_be() {
    let _ = open_vault();
    lock();
    let before = std::fs::read(path()).unwrap();
    // Settings: the passcode there is was right; the new one is made.
    rewrap_begin_at(path(), "2468");
    let (new_phc, _, _) = crate::passcode::make("97531").unwrap();
    let kept: Result<(), std::io::Error> = rewrap_with("97531", &new_phc, || crate::passcode::keep(&new_phc));
    kept.unwrap();
    assert!(under_current_passcode());
    assert!(matches!(wait(begin_unlock(Zeroizing::new("2468".into()))), Outcome::Wrong), "the old passcode no longer opens it");
    opened(wait(begin_unlock(Zeroizing::new("97531".into()))));
    assert_eq!(with(|e| e[0].password.clone()), Some(PASSWORD.to_owned()), "the same entries");
    lock();
    assert_ne!(std::fs::read(path()).unwrap()[..super::crypto::HEAD], before[..super::crypto::HEAD]);

    // Keeping the new passcode fails: the vault is put back as it was.
    let as_it_was = std::fs::read(path()).unwrap();
    rewrap_begin_at(path(), "97531");
    let (other, _, _) = crate::passcode::make("86420").unwrap();
    let failed: Result<(), &str> = rewrap_with("86420", &other, || Err("the card is full"));
    assert_eq!(failed, Err("the card is full"));
    assert_eq!(std::fs::read(path()).unwrap(), as_it_was);
    opened(wait(begin_unlock(Zeroizing::new("97531".into()))));

    // Given up halfway: nothing changes, and the held key is wiped.
    lock();
    rewrap_begin_at(path(), "97531");
    rewrap_cancel();
    assert!(pending().is_none());
    // A wrong passcode there holds nothing.
    rewrap_begin_at(path(), "0000");
    assert!(pending().is_none());
}

#[test]
fn a_vault_under_an_earlier_passcode_says_so_and_moves_when_that_is_typed_again() {
    let _ = open_vault();
    lock();
    // The passcode taken off over SSH and set again, to the same digits.
    let (phc, _, _) = crate::passcode::make("2468").unwrap();
    crate::passcode::keep(&phc).unwrap();
    assert!(!under_current_passcode());
    opened(wait(begin_unlock(Zeroizing::new("2468".into()))));
    assert!(under_current_passcode(), "moved to the passcode set now");
}

/// The open vault last used `ago`.
fn used_ago(ago: Duration) {
    let mut s = session();
    let s = s.as_mut().expect("open");
    s.used = std::time::Instant::now().checked_sub(ago).expect("the clock goes back that far");
}

#[test]
fn it_locks_after_two_idle_minutes_with_the_screen_or_the_lock_and_on_leaving() {
    let _ = open_vault();
    watch(false);
    assert!(is_open(), "in use");
    // Not quite two minutes without a key; then two.
    used_ago(IDLE - Duration::from_secs(1));
    watch(false);
    assert!(is_open());
    used_ago(IDLE);
    watch(false);
    assert!(!is_open(), "idle two minutes");
    // The screen off, StandBy, the lock screen.
    opened(wait(begin_unlock(Zeroizing::new("2468".into()))));
    watch(true);
    assert!(!is_open());
    // Leaving the app: set aside (another app from the recents), or closed.
    opened(wait(begin_unlock(Zeroizing::new("2468".into()))));
    let mut app = Vault::new();
    app.set_aside();
    assert!(!is_open(), "left for another app");
    opened(wait(begin_unlock(Zeroizing::new("2468".into()))));
    drop(app);
    assert!(!is_open(), "the app closed");
    // A page inside it let go (back from an entry) is not leaving it.
    opened(wait(begin_unlock(Zeroizing::new("2468".into()))));
    let id = with(|e| e[0].id).unwrap();
    let mut page = EntryPage::new(id);
    page.set_aside();
    drop(page);
    assert!(is_open());
    // A key keeps it open.
    used_ago(IDLE - Duration::from_secs(5));
    touch();
    watch(false);
    assert!(is_open() && locks_in().unwrap() > 100);
}

#[test]
fn an_import_comes_in_and_a_backup_goes_out_sealed_and_comes_back() {
    let _ = open_vault();
    let dir = crate::passcode::file().parent().unwrap().join("storage");
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("bitwarden.csv");
    std::fs::write(&csv, format!("folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n,,login,Scouting,,,0,https://scout.example.org,scout,{PASSWORD}-2,\n")).unwrap();
    let found = import::read_file(&csv, 0).unwrap();
    let mut counts = (0, 0);
    change(|entries| counts = import::merge(entries, found.entries)).unwrap();
    assert_eq!(counts, (1, 0));
    import::shred(&csv).unwrap();
    assert!(!csv.exists());
    // A backup, sealed under its own password.
    let dest = import::backup_path(&dir, "2026-11-21");
    let made = wait(begin_backup(Zeroizing::new("a long backup phrase".into()), dest.clone()).unwrap()).unwrap();
    assert_eq!(made, dest);
    let file = std::fs::read(&dest).unwrap();
    assert!(!file.windows(PASSWORD.len()).any(|w| w == PASSWORD.as_bytes()));
    assert!(matches!(wait(begin_restore(Zeroizing::new("not it".into()), dest.clone())), Err(crypto::Error::Wrong)));
    let back = wait(begin_restore(Zeroizing::new("a long backup phrase".into()), dest.clone())).unwrap();
    assert_eq!(back.len(), 2);
    let mut counts = (0, 0);
    change(|entries| counts = import::merge(entries, back)).unwrap();
    assert_eq!(counts, (0, 2), "restored into the vault it came from: all there already");
    // A backup is never mistaken for the vault.
    flush();
    std::fs::copy(&dest, path()).unwrap();
    lock();
    assert!(matches!(wait(begin_unlock(Zeroizing::new("2468".into()))), Outcome::Failed(_)));
}

/// Drive a vault page as the shell would: every message the island would
/// say is gathered.
struct Harness {
    robot: crate::robot::Robot,
    settings: Settings,
    said: Vec<String>,
    stack: Vec<Box<dyn Screen>>,
    g: Gfx,
}

impl Harness {
    fn new(first: Box<dyn Screen>) -> Harness {
        Harness { robot: crate::robot::Robot::demo(), settings: Settings::default(), said: Vec::new(), stack: vec![first], g: Gfx::new(bezel::Tokens::dark()) }
    }

    fn follow(&mut self, nav: Nav) {
        match nav {
            Nav::Push(s) | Nav::Open(s, _) => self.stack.push(s),
            Nav::Pop | Nav::Done => {
                if let Some(mut s) = self.stack.pop() {
                    s.set_aside();
                }
                if let Some(top) = self.stack.last_mut() {
                    top.resume();
                }
            }
            _ => {}
        }
    }

    fn key(&mut self, key: Key) {
        let mut say = None;
        let top = self.stack.last_mut().expect("a page");
        let nav = top.key(key, &mut Cx { robot: &mut self.robot, settings: &mut self.settings, say: &mut say, at_rest: true });
        self.said.extend(say);
        self.follow(nav);
        self.frame();
    }

    fn frame(&mut self) {
        let mut say = None;
        if let Some(top) = self.stack.last_mut() {
            let nav = top.update(&mut Cx { robot: &mut self.robot, settings: &mut self.settings, say: &mut say, at_rest: true });
            self.said.extend(say);
            self.follow(nav);
        }
        if let Some(top) = self.stack.last_mut() {
            top.tick(1.0 / 60.0);
            self.g.foot = H;
            self.g.viewports.clear();
            top.draw(&mut self.g, &self.robot);
        }
    }

    fn title(&self) -> String {
        self.stack.last().map(|s| s.title().to_owned()).unwrap_or_default()
    }
}

#[test]
fn secrets_never_reach_the_island_the_logs_the_clipboards_history_or_spotlight() {
    let _ = open_vault();
    let id = with(|e| e[0].id).unwrap();
    let mut h = Harness::new(Box::new(EntryPage::new(id)));
    h.frame();
    // Every field copied, the password shown and hidden, the notes read.
    for key in [Key::Enter, Key::Down, Key::Enter, Key::Char(' '), Key::Char(' '), Key::Down, Key::Enter, Key::Down, Key::Enter, Key::Down, Key::Enter, Key::Back] {
        h.key(key);
    }
    let copied = clip::tests::copied();
    assert!(copied.iter().any(|(t, _)| t == PASSWORD) && copied.iter().any(|(t, _)| t == USERNAME), "copied: {}", copied.len());
    assert!(copied.iter().all(|(_, s)| *s == clip::Sensitivity::Secret), "every copy is a secret");
    assert!(clip::history().is_empty(), "and none of them is in the history");
    for said in &h.said {
        for secret in [PASSWORD, USERNAME, NOTE, TOTP_SECRET] {
            assert!(!said.contains(secret), "the island said {said:?}");
        }
    }
    assert!(h.said.iter().any(|s| s.starts_with("password copied")), "{:?}", h.said);
    // Spotlight: titles only, and nothing once locked.
    use crate::spotlight::Provider;
    let finder = Finder;
    let robot = crate::robot::Robot::demo();
    for typed in ["git", "pit", "Tr0ub", "locker", "github.com"] {
        let mut out = Vec::new();
        finder.answer(&crate::spotlight::Query::new(typed), &robot, &mut out);
        for item in &out {
            let shown = format!("{} {} {} {}", item.id, item.title, item.detail, item.words);
            for secret in [PASSWORD, USERNAME, NOTE, TOTP_SECRET, "github.com"] {
                assert!(!shown.contains(secret), "{typed}: {shown}");
            }
        }
        assert_eq!(out.len(), usize::from(typed == "git"), "{typed}: found by its title alone");
    }
    lock();
    let mut out = Vec::new();
    finder.answer(&crate::spotlight::Query::new("git"), &robot, &mut out);
    assert!(out.is_empty(), "nothing while it is locked");
    // The vault never writes to the log: no line of it prints.
    for (name, source) in [("mod.rs", include_str!("mod.rs")), ("screen.rs", include_str!("screen.rs")), ("crypto.rs", include_str!("crypto.rs")), ("entry.rs", include_str!("entry.rs")), ("import.rs", include_str!("import.rs")), ("totp.rs", include_str!("totp.rs")), ("clip.rs", include_str!("clip.rs")), ("gen.rs", include_str!("gen.rs"))] {
        for bad in ["eprintln!", "println!", "dbg!", "log::"] {
            assert!(!source.contains(bad), "{name} has {bad}");
        }
    }
}

#[test]
fn an_entry_masks_its_password_until_space_and_masks_it_again_on_leaving() {
    let _ = open_vault();
    let id = with(|e| e[0].id).unwrap();
    // Without its code, whose ring moves with the clock between pictures.
    change(|entries| entries[0].set(Field::Totp, "")).unwrap();
    let mut page = EntryPage::new(id);
    let mut g = Gfx::new(bezel::Tokens::dark());
    let robot = crate::robot::Robot::demo();
    let mut settings = Settings::default();
    let mut robot_mut = crate::robot::Robot::demo();
    let mut say = None;
    let text = |g: &mut Gfx, page: &mut EntryPage| {
        g.canvas.clear(bezel::Color::BLACK);
        page.draw(g, &robot);
        g.canvas.px.clone()
    };
    let masked = text(&mut g, &mut page);
    page.key(Key::Char(' '), &mut Cx { robot: &mut robot_mut, settings: &mut settings, say: &mut say, at_rest: true });
    let shown = text(&mut g, &mut page);
    assert_ne!(masked, shown, "space shows it");
    page.set_aside();
    assert_eq!(text(&mut g, &mut page), masked, "masked again once left");
    page.key(Key::Char(' '), &mut Cx { robot: &mut robot_mut, settings: &mut settings, say: &mut say, at_rest: true });
    page.resume();
    assert_eq!(text(&mut g, &mut page), masked, "and on coming back to it");
}

#[test]
fn every_page_fits_whole_at_every_text_size_in_both_looks() {
    crate::shell::tests::sandbox();
    let _ = open_vault();
    let id = with(|e| e[0].id).unwrap();
    let pages: Vec<(&str, fn(u64) -> Box<dyn Screen>, Vec<Key>)> = vec![
        ("list", |_| Box::new(Vault::new()), vec![]),
        ("search", |_| Box::new(Vault::new()), vec![Key::Char('/'), Key::Char('g')]),
        ("entry", |id| Box::new(EntryPage::new(id)), vec![]),
        ("entry-code", |id| Box::new(EntryPage::new(id)), vec![Key::Down, Key::Down]),
        ("edit", |id| Box::new(super::screen::EditPage::new(Some(id))), vec![]),
        ("new", |_| Box::new(super::screen::EditPage::new(None)), vec![Key::Enter]),
        ("generator", |_| Box::new(super::screen::GeneratorPage::new(true)), vec![]),
        ("tools", |_| Box::new(super::screen::ToolsPage::new()), vec![]),
    ];
    for look in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            for (name, make, keys) in &pages {
                // The app's page let go last time locked it.
                if !is_open() {
                    opened(wait(begin_unlock(Zeroizing::new("2468".into()))));
                }
                let mut h = Harness::new(make(id));
                h.g.theme = look;
                h.g.set_text_size(size);
                h.frame();
                for key in keys {
                    h.key(*key);
                }
                h.frame();
                assert!(h.g.missing_icons().is_empty(), "{name}: {:?}", h.g.missing_icons());
                for &(top, bottom) in &h.g.viewports {
                    assert!((bottom - h.g.foot).abs() < 0.5, "{name} {look:?} {size:?}: the list from {top} ends at {bottom}, the foot at {}", h.g.foot);
                }
                assert!(!h.title().is_empty());
                assert!(h.g.measure(&h.title(), h.g.ty.title) <= h.g.measure("Appearance", h.g.ty.title) + 9.0, "{} too wide a title", h.title());
            }
        }
    }
}
