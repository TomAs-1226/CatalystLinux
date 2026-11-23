use std::collections::BTreeMap;
use std::path::PathBuf;

use super::form::{Form, Kind, Metric};
use super::practice::{Session, Split};
use super::share::{self, Inbox};
use super::stats::{self, Picklist};
use super::store::{self, Merged, Record, Station, Store, Version};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("catalyst-scout-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn values(pairs: &[(&str, f32)]) -> BTreeMap<String, f32> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}

fn record(n: u32, team: u32, station: u8, pairs: &[(&str, f32)], updated: i64) -> Record {
    Record {
        event: "2026test".into(),
        match_key: format!("2026test_qm{n}"),
        label: format!("Q{n}"),
        team,
        station: Station(station),
        form: "rebuilt-2026".into(),
        values: values(pairs),
        scout: "maya".into(),
        by: "maya".into(),
        updated,
        ..Record::default()
    }
}

// ------------------------------------------------------------------- form ---

#[test]
fn the_shipped_form_reads_with_its_sections_fields_and_totals() {
    let f = Form::shipped();
    assert_eq!(f.id, "rebuilt-2026");
    assert_eq!(f.season, 2026);
    let ids: Vec<&str> = f.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["auto", "teleop", "endgame", "match", "after"]);
    assert_eq!(f.match_seconds(), 160.0, "REBUILT's 20 s AUTO and 2:20 TELEOP");
    assert_eq!(f.pages().len(), 4, "auto, teleop, end game, after");
    assert!(f.fields.iter().any(|x| matches!(x.kind, Kind::Text(_))), "notes");
    assert_eq!(f.totals[0].id, "points");
}

#[test]
fn keys_add_and_shift_or_the_key_beneath_takes_away_in_the_phase_they_work_in() {
    let f = Form::shipped();
    let col = |(i, by): (usize, i32)| (f.fields[i].column(&f), by);
    let (auto, teleop, endgame, after) = (0, 1, 2, 4);
    assert_eq!(f.key_in(auto, 'q').map(col), Some(("auto.fuel".into(), 1)));
    assert_eq!(f.key_in(auto, 'Q').map(col), Some(("auto.fuel".into(), -1)));
    assert_eq!(f.key_in(auto, 'a').map(col), Some(("auto.fuel".into(), -1)));
    assert_eq!(f.key_in(teleop, 'q').map(col), Some(("teleop.fuel".into(), 1)));
    // End game goes on counting TELEOP's fuel, and has its tower.
    assert_eq!(f.key_in(endgame, 'q').map(col), Some(("teleop.fuel".into(), 1)));
    assert_eq!(f.key_in(endgame, 't').map(col), Some(("endgame.tower".into(), 1)));
    assert_eq!(f.key_in(teleop, 't'), None, "the tower is the end game's");
    assert_eq!(f.key_in(auto, 'e'), None, "nothing is passed in AUTO");
    // Fouls count in every phase, and after.
    for s in [auto, teleop, endgame, after] {
        assert_eq!(f.key_in(s, 'y').map(col), Some(("match.minor".into(), 1)));
    }
    assert_eq!(f.key_in(auto, '1'), None);
}

#[test]
fn a_form_that_contradicts_itself_is_refused() {
    let base = |fields: &str| format!(r#"{{"id":"x","sections":[{{"id":"auto","seconds":15}},{{"id":"tele","seconds":135,"inherits":"auto"}}],"fields":[{fields}]}}"#);
    assert!(Form::parse(&base(r#"{"section":"auto","id":"a","key":"q"}"#)).is_ok());
    let twice = base(r#"{"section":"auto","id":"a","key":"q"},{"section":"tele","id":"b","key":"q"}"#);
    assert!(Form::parse(&twice).unwrap_err().contains("key q"), "an inherited key used again");
    assert!(Form::parse(&base(r#"{"section":"nowhere","id":"a"}"#)).is_err());
    assert!(Form::parse(&base(r#"{"section":"auto","id":"a","key":"Q"}"#)).is_err(), "Shift is for taking away");
    assert!(Form::parse(&base(r#"{"section":"auto","id":"a","key":"1"}"#)).is_err(), "digits rate");
    assert!(Form::parse(&base(r#"{"section":"auto","id":"a","kind":"choice","options":["only"]}"#)).is_err());
    assert!(Form::parse("not json").is_err());
}

#[test]
fn points_add_up_by_the_game_manual() {
    let f = Form::shipped();
    let v = values(&[("auto.fuel", 8.0), ("auto.missed", 3.0), ("auto.tower", 1.0), ("teleop.fuel", 30.0), ("teleop.passed", 12.0), ("endgame.tower", 2.0), ("match.minor", 2.0), ("match.major", 1.0)]);
    let total = |id: &str| f.total(f.totals.iter().find(|t| t.id == id).unwrap(), &v);
    assert_eq!(total("auto_points"), 8.0 + 15.0, "fuel 1 each, tower level 1 in AUTO 15");
    assert_eq!(total("teleop_points"), 30.0, "passed and missed fuel score nothing");
    assert_eq!(total("endgame_points"), 20.0, "level 2");
    assert_eq!(total("points"), 73.0);
    assert_eq!(total("foul_points"), 2.0 * 5.0 + 15.0);
    // Steps: a count never below nothing, a choice goes round, a toggle flips.
    let field = |c: &str| f.fields.iter().find(|x| x.column(&f) == c).unwrap();
    assert_eq!(field("auto.fuel").stepped(0.0, -1), 0.0);
    assert_eq!(field("endgame.tower").stepped(4.0, 1), 0.0);
    assert_eq!(field("endgame.tower").stepped(0.0, -1), 4.0);
    assert_eq!(field("endgame.tower").shown(3.0), "L3");
    assert_eq!(field("auto.tower").stepped(1.0, 1), 0.0);
    assert_eq!(field("after.driving").stepped(5.0, 1), 5.0);
    // A rating not given is no rating, not a zero.
    let driving = f.metrics().into_iter().find(|m| matches!(m, Metric::Field(i) if f.fields[*i].id == "driving")).unwrap();
    assert_eq!(f.metric_value(&driving, &v), None);
    assert_eq!(f.metric_value(&driving, &values(&[("after.driving", 4.0)])), Some(4.0));
}

// ---------------------------------------------------------------- records ---

#[test]
fn records_are_kept_one_a_team_a_match_and_read_back() {
    let root = scratch("store");
    let mut s = Store::load(&root);
    let mut r = record(3, 254, 0, &[("auto.fuel", 4.0)], 100);
    r.texts.insert("after.notes".into(), "fast, \"clean\" intake".into());
    r.stamp(100, "maya", "scouted".into());
    s.put(r.clone()).unwrap();
    s.put(record(1, 1678, 1, &[], 90)).unwrap();
    // The same team in the same match again replaces it.
    let mut again = r.clone();
    again.values.insert("auto.fuel".into(), 5.0);
    s.put(again).unwrap();
    assert_eq!(s.records.len(), 2);
    let back = Store::load(&root);
    assert_eq!(back.records.len(), 2);
    assert_eq!(back.records[0].match_key, "2026test_qm1", "in match order");
    let got = back.get("2026test_qm3", 254).unwrap();
    assert_eq!(got.value("auto.fuel"), 5.0);
    assert_eq!(got.texts["after.notes"], "fast, \"clean\" intake");
    assert_eq!(got.history, vec![Version { at: 100, by: "maya".into(), change: "scouted".into() }]);
    assert_eq!(got.station.label(), "red 1");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn merging_adds_once_and_the_newer_edit_wins() {
    let mut here = record(5, 971, 3, &[("teleop.fuel", 10.0)], 1000);
    here.history.push(Version { at: 1000, by: "maya".into(), change: "scouted".into() });
    // Nothing here: added.
    assert_eq!(store::merge(None, &here).0, Merged::Added);
    // The same version back: kept, unchanged, history and all.
    let (what, out) = store::merge(Some(&here), &here.clone());
    assert_eq!((what, &out), (Merged::Kept, &here));
    // Newer elsewhere: it wins, and the histories join.
    let mut newer = here.clone();
    newer.values.insert("teleop.fuel".into(), 12.0);
    newer.stamp(2000, "sam", "teleop fuel in 10 → 12".into());
    let (what, out) = store::merge(Some(&here), &newer);
    assert_eq!(what, Merged::Updated);
    assert_eq!(out.value("teleop.fuel"), 12.0);
    assert_eq!(out.history.len(), 2);
    // Older elsewhere: what is here stays.
    let (what, out) = store::merge(Some(&newer), &here);
    assert_eq!(what, Merged::Kept);
    assert_eq!(out.value("teleop.fuel"), 12.0);
    // Two edits at the same second: every handheld picks the same one.
    let mut a = here.clone();
    a.updated = 3000;
    a.by = "ana".into();
    a.values.insert("teleop.fuel".into(), 1.0);
    let mut b = a.clone();
    b.by = "ben".into();
    b.values.insert("teleop.fuel".into(), 2.0);
    assert_eq!(store::merge(Some(&a), &b).1.value("teleop.fuel"), 2.0);
    assert_eq!(store::merge(Some(&b), &a).1.value("teleop.fuel"), 2.0);
}

#[test]
fn match_keys_sort_in_the_order_matches_are_played() {
    let mut keys = vec!["e_f1m1", "e_qm10", "e_sf2m1", "e_qm2", "e_qf1m3"];
    keys.sort_by_key(|k| store::match_order(k));
    assert_eq!(keys, ["e_qm2", "e_qm10", "e_qf1m3", "e_sf2m1", "e_f1m1"]);
    assert_eq!(share::label_of("2026nyro_qm31"), "Q31");
    assert_eq!(share::label_of("2026nyro_sf4m1"), "M4");
    assert_eq!(Station::parse("b3"), Some(Station(5)));
    assert_eq!(Station::parse("red 2"), Some(Station(1)));
    assert_eq!(Station(5).next(1), Station(0));
}

// ------------------------------------------------------------------ codes ---

fn demo_records(n: u32) -> Vec<Record> {
    (1..=n)
        .map(|i| {
            let mut r = record(i / 6 + 1, 100 + i, (i % 6) as u8, &[("auto.fuel", (i % 9) as f32), ("teleop.fuel", (i * 7 % 40) as f32), ("endgame.tower", (i % 4) as f32), ("after.driving", (i % 5 + 1) as f32)], 1_763_000_000 + i as i64);
            if i % 3 == 0 {
                r.texts.insert("after.notes".into(), format!("note {i}: shoots, from \"far\"; slow intake, {}", "x".repeat(i as usize % 150)));
            }
            r
        })
        .collect()
}

#[test]
fn codes_carry_every_record_whole_and_read_back_the_same() {
    let f = Form::shipped();
    let records = demo_records(40);
    let refs: Vec<&Record> = records.iter().collect();
    let codes = share::chunks(&f, "2026test", &refs);
    assert!(codes.len() > 4, "40 records need several codes: {}", codes.len());
    let mut back = Vec::new();
    for (i, text) in codes.iter().enumerate() {
        assert!(text.len() <= share::CODE_BYTES, "code {i} is {} bytes", text.len());
        // Small enough to draw at two pixels a module on the panel.
        let qr = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Low).unwrap();
        assert!(qr.size() <= 61, "code {i} is {} modules", qr.size());
        let chunk = share::parse(&f, text).unwrap();
        assert_eq!((chunk.index, chunk.total), (i + 1, codes.len()));
        back.extend(chunk.records);
    }
    assert_eq!(back.len(), records.len());
    for (a, b) in records.iter().zip(&back) {
        assert_eq!((a.match_key.as_str(), a.team, a.station, a.updated, a.scout.as_str()), (b.match_key.as_str(), b.team, b.station, b.updated, b.scout.as_str()));
        let nonzero = |r: &Record| r.values.iter().filter(|(_, v)| **v != 0.0).map(|(k, v)| (k.clone(), *v)).collect::<Vec<_>>();
        assert_eq!(nonzero(a), nonzero(b));
        assert!(a.same_content(b) || a.texts.get("after.notes").is_some_and(|n| n.len() > 120));
        let notes = |r: &Record| r.texts.get("after.notes").cloned().unwrap_or_default();
        assert!(notes(a).starts_with(&notes(b)) && notes(b).len() >= notes(a).len().min(120), "notes travel, cut to 120 at most");
    }
    // Another form's codes are told apart.
    let other = Form::parse(&share_form_with_extra_field()).unwrap();
    assert_eq!(share::parse(&other, &codes[0]).unwrap_err(), "made with another form");
    assert!(share::parse(&f, "hello").is_err());
}

fn share_form_with_extra_field() -> String {
    super::form::SHIPPED.replace(r#"{ "section": "auto", "id": "missed""#, r#"{ "section": "auto", "id": "left", "kind": "toggle" }, { "section": "auto", "id": "missed""#)
}

#[test]
fn reading_codes_twice_or_in_any_order_adds_each_record_once() {
    let root = scratch("inbox");
    let f = Form::shipped();
    let records = demo_records(30);
    let refs: Vec<&Record> = records.iter().collect();
    let codes = share::chunks(&f, "2026test", &refs);
    let mut store = Store::load(&root);
    let mut inbox = Inbox::default();
    for text in codes.iter().rev().chain(codes.iter()) {
        inbox.take(share::parse(&f, text).unwrap(), &mut store);
    }
    assert_eq!(store.records.len(), 30);
    assert_eq!((inbox.added, inbox.updated), (30, 0));
    assert_eq!(inbox.progress(), format!("all {} codes read", codes.len()));
    // Read again, as a new set (the same codes, a fresh inbox): nothing new.
    let mut again = Inbox::default();
    for text in &codes {
        again.take(share::parse(&f, text).unwrap(), &mut store);
    }
    assert_eq!((again.added, again.updated), (0, 0));
    assert_eq!(again.outcome(), "nothing new");
    assert_eq!(Store::load(&root).records.len(), 30);
    // An edit made since, shared back, wins.
    let mut edited = records[4].clone();
    edited.values.insert("teleop.fuel".into(), 99.0);
    edited.updated += 50;
    edited.by = "sam".into();
    let code = share::chunks(&f, "2026test", &[&edited]);
    let mut third = Inbox::default();
    third.take(share::parse(&f, &code[0]).unwrap(), &mut store);
    assert_eq!((third.added, third.updated), (0, 1));
    assert_eq!(store.get(&edited.match_key, edited.team).unwrap().value("teleop.fuel"), 99.0);
    assert_eq!(store.get(&edited.match_key, edited.team).unwrap().by, "sam");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn the_csv_has_a_column_for_every_field_and_total_and_quotes_what_it_must() {
    let f = Form::shipped();
    let mut r = record(7, 5805, 2, &[("auto.fuel", 6.0), ("auto.tower", 1.0), ("teleop.fuel", 20.0)], 1_763_000_000);
    r.texts.insert("after.notes".into(), "fast, \"clean\"\nintake".into());
    let text = share::csv(&f, &[&r]);
    let mut lines = text.lines();
    let head = share::cells(lines.next().unwrap());
    assert_eq!(&head[..4], ["event", "match", "team", "station"]);
    assert!(head.contains(&"auto_fuel".to_string()) && head.contains(&"after_notes".to_string()) && head.contains(&"points".to_string()));
    let row = share::cells(lines.next().unwrap());
    assert_eq!(row.len(), head.len());
    let at = |name: &str| &row[head.iter().position(|h| h == name).unwrap()];
    assert_eq!(at("team"), "5805");
    assert_eq!(at("station"), "r3");
    assert_eq!(at("auto_fuel"), "6");
    assert_eq!(at("teleop_missed"), "0");
    assert_eq!(at("after_notes"), "fast, \"clean\" intake");
    assert_eq!(at("points"), "41");
    assert!(lines.next().is_none());
    let root = scratch("csv");
    let path = share::export_csv(&root, &f, "2026test", &[&r]).unwrap();
    assert!(path.ends_with("export/2026test.csv") || path.ends_with("export\\2026test.csv"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    std::fs::remove_dir_all(&root).ok();
}

// ------------------------------------------------------------ teams, picks ---

#[test]
fn teams_average_their_matches_and_sort_by_any_metric() {
    let f = Form::shipped();
    let records = [
        record(1, 254, 0, &[("teleop.fuel", 40.0), ("match.minor", 2.0)], 1),
        record(2, 254, 0, &[("teleop.fuel", 20.0), ("after.driving", 5.0)], 2),
        record(1, 1678, 1, &[("teleop.fuel", 35.0)], 3),
        record(3, 118, 2, &[("teleop.fuel", 10.0)], 4),
    ];
    let refs: Vec<&Record> = records.iter().collect();
    let mut teams = stats::teams(&f, &refs);
    let points = stats::metric_index(&f, "points").unwrap();
    stats::sort_by(&f, &mut teams, points);
    assert_eq!(teams.iter().map(|t| t.team).collect::<Vec<_>>(), [1678, 254, 118]);
    assert_eq!(teams[1].avg(points), Some(30.0));
    assert_eq!(teams[1].matches, ["Q1", "Q2"]);
    // Fouls: fewest first.
    let fouls = stats::metric_index(&f, "foul_points").unwrap();
    stats::sort_by(&f, &mut teams, fouls);
    assert_eq!(teams.last().unwrap().team, 254);
    // A rating counts only where it was given.
    let driving = stats::metric_index(&f, "after.driving").unwrap();
    stats::sort_by(&f, &mut teams, driving);
    assert_eq!(teams[0].team, 254);
    assert_eq!(teams[0].avg(driving), Some(5.0));
    assert_eq!(teams[1].avg(driving), None);
    assert_eq!(stats::scale(&teams, points), 40.0);
}

#[test]
fn the_picklist_moves_by_keys_and_keeps_teams_not_to_pick_below() {
    let mut p = Picklist { event: "2026test".into(), picks: Vec::new() };
    assert!(p.add_missing(&[254, 1678, 118, 971]));
    let order = |p: &Picklist| p.picks.iter().map(|x| x.team).collect::<Vec<_>>();
    assert_eq!(order(&p), [254, 1678, 118, 971]);
    assert_eq!(p.shift(2, -1), 1);
    assert_eq!(order(&p), [254, 118, 1678, 971]);
    assert_eq!(p.shift(0, -1), 0, "the top stays the top");
    assert_eq!(p.to_top(3), 0);
    assert_eq!(order(&p), [971, 254, 118, 1678]);
    // Not to pick: to the top of those below the picks.
    assert_eq!(p.toggle_dnp(1), 3);
    assert_eq!(order(&p), [971, 118, 1678, 254]);
    assert_eq!(p.rank(3), None);
    assert_eq!(p.rank(2), Some(3));
    // Picks move only among picks.
    assert_eq!(p.shift(2, 1), 2);
    assert_eq!(p.toggle_dnp(0), 2);
    assert_eq!(order(&p), [118, 1678, 971, 254]);
    // Back on the list: to the foot of the picks.
    assert_eq!(p.toggle_dnp(3), 2);
    assert_eq!(order(&p), [118, 1678, 254, 971]);
    // New teams join above those not to pick.
    p.add_missing(&[118, 6328]);
    assert_eq!(order(&p), [118, 1678, 254, 6328, 971]);
    p.picks[0].note = "best defender".into();
    let root = scratch("picklist");
    p.save(&root).unwrap();
    assert_eq!(Picklist::load(&root, "2026test"), p);
    std::fs::remove_dir_all(&root).ok();
}

// --------------------------------------------------------------- practice ---

#[test]
fn practice_sums_up_each_drivers_cycles() {
    let split = |d: &str, s: f32| Split { driver: d.into(), secs: s };
    let s = Session { started: 1, splits: vec![split("maya", 10.0), split("sam", 8.0), split("maya", 12.0), split("sam", 9.0)] };
    assert_eq!(s.drivers(), ["maya", "sam"]);
    let all = s.summary(None).unwrap();
    assert_eq!((all.cycles, all.mean, all.best), (4, 9.75, 8.0));
    let maya = s.summary(Some("maya")).unwrap();
    assert_eq!((maya.cycles, maya.mean, maya.best), (2, 11.0, 10.0));
    assert!(s.summary(Some("ana")).is_none());
    assert_eq!(Session::from_json(&s.to_json()), Some(s));
}

// ------------------------------------------------------------------ screen ---

#[test]
fn each_screens_keys_fit_the_sheet_in_each_state() {
    use crate::platform::Key;
    use crate::shell::Screen;
    crate::shell::tests::sandbox();
    let mut g = crate::gfx::Gfx::new(bezel::Tokens::dark());
    let mut robot = crate::robot::Robot::none();
    let mut settings = crate::shell::Settings::default();
    let mut say = None;
    let mut cx = crate::shell::Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
    let mut fits = |s: &dyn Screen| {
        let groups = s.key_sheet();
        assert!(!groups.is_empty(), "{}: no keys", s.title());
        crate::keysheet::fits(&mut g, &groups).unwrap();
    };
    let mut walk = |s: &mut dyn Screen, keys: &[Key], cx: &mut crate::shell::Cx| {
        fits(s);
        for k in keys {
            s.key(*k, cx);
            fits(s);
        }
    };
    // Home: the next match, the list, the name typed.
    walk(&mut super::home::Scout::new(), &[Key::Right, Key::End, Key::Enter], &mut cx);
    walk(&mut super::home::Matches::new(Some(super::event::demo()), Station(1)), &[], &mut cx);
    walk(&mut super::home::ByHand::new(Station(4)), &[Key::Char('4'), Key::Enter], &mut cx);
    // The form: before the clock, running, then after the match, on each row.
    let mut form = super::scouting::Scouting::new("2026sheet", "2026sheet_qm1", "Q1", 6328, Station(4));
    walk(&mut form, &[Key::Char(' '), Key::Right, Key::Right, Key::Right, Key::Down, Key::Down, Key::Down, Key::Enter], &mut cx);
    walk(&mut super::practice::Practice::new(), &[Key::Char('n')], &mut cx);
    walk(&mut super::practice::Sessions::new(), &[], &mut cx);
    walk(&mut super::teams::Teams::new("2026sheet".into()), &[], &mut cx);
    walk(&mut super::teams::TeamPage::new("2026sheet".into(), 6328), &[], &mut cx);
    walk(&mut super::teams::PicklistScreen::new("2026sheet".into()), &[Key::Char('n')], &mut cx);
    walk(&mut super::sharing::ShareMenu::new("2026sheet".into(), None), &[Key::Down, Key::Down, Key::Down], &mut cx);
    walk(&mut super::sharing::Codes::new("Event".into(), vec!["a".into(), "b".into()]), &[Key::Char(' ')], &mut cx);
}

#[test]
fn scouting_a_match_on_the_keys_keeps_one_record_with_its_history() {
    use crate::platform::Key;
    use crate::shell::Screen;
    crate::shell::tests::sandbox();
    let mut screen = super::scouting::Scouting::new("2026keys", "2026keys_qm4", "Q4", 6328, Station(4));
    let mut robot = crate::robot::Robot::none();
    let mut settings = crate::shell::Settings::default();
    let mut say = None;
    let mut cx = crate::shell::Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
    // AUTO: three fuel, one taken back, the tower; the clock starts with the first.
    for c in "qqqQr".chars() {
        screen.key(Key::Char(c), &mut cx);
    }
    // On to TELEOP by hand: fuel, a pass, a foul, then one undone.
    screen.key(Key::Right, &mut cx);
    for c in "qqeyq".chars() {
        screen.key(Key::Char(c), &mut cx);
    }
    screen.key(Key::Backspace, &mut cx);
    // End game: level 3. After: driving rated 4.
    screen.key(Key::Right, &mut cx);
    for _ in 0..3 {
        screen.key(Key::Char('t'), &mut cx);
    }
    screen.key(Key::Right, &mut cx);
    screen.key(Key::Down, &mut cx);
    screen.key(Key::Char('4'), &mut cx);
    screen.key(Key::Back, &mut cx);
    let r = super::with_store(|s| s.get("2026keys_qm4", 6328).cloned()).expect("kept");
    let v = |c: &str| r.value(c);
    assert_eq!((v("auto.fuel"), v("auto.tower"), v("teleop.fuel"), v("teleop.passed"), v("match.minor")), (2.0, 1.0, 2.0, 1.0, 1.0));
    assert_eq!((v("endgame.tower"), v("after.driving")), (3.0, 4.0));
    assert_eq!(r.station, Station(4));
    assert_eq!(r.history.len(), 1);
    assert_eq!(r.history[0].change, "scouted");
    // Opened again and changed: the history says what.
    let mut screen = super::scouting::Scouting::new("2026keys", "2026keys_qm4", "Q4", 6328, Station(4));
    screen.key(Key::Left, &mut cx);
    screen.key(Key::Char('g'), &mut cx);
    screen.key(Key::Back, &mut cx);
    let r = super::with_store(|s| s.get("2026keys_qm4", 6328).cloned()).unwrap();
    assert_eq!(r.history.len(), 2);
    assert_eq!(r.history[1].change, "end game tower L3 → L2");
    // Opened and left as it was: nothing more.
    let mut screen = super::scouting::Scouting::new("2026keys", "2026keys_qm4", "Q4", 6328, Station(4));
    screen.key(Key::Back, &mut cx);
    assert_eq!(super::with_store(|s| s.get("2026keys_qm4", 6328).unwrap().history.len()), 2);
    // A match opened and left with nothing in it is not kept.
    let mut screen = super::scouting::Scouting::new("2026keys", "2026keys_qm5", "Q5", 254, Station(0));
    screen.key(Key::Back, &mut cx);
    assert!(super::with_store(|s| s.get("2026keys_qm5", 254).is_none()));
}
