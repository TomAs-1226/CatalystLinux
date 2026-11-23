use super::health::{analyse, health, synthetic, Measure, Reading};
use super::*;

const NOW: i64 = 1_795_000_000;

fn reading_of(bytes: &[u8]) -> Reading {
    analyse(bytes).unwrap().expect("a reading")
}

// ---------------------------------------------------------------- health ---

#[test]
fn the_resistance_is_found_from_volts_and_current() {
    for (mohm, seed) in [(14.0, 1), (22.0, 2), (31.0, 3)] {
        let r = reading_of(&synthetic(12.9, mohm, 1.0, true, seed));
        let found = r.mohm.expect("current is logged");
        assert!((found - mohm).abs() <= mohm * 0.1, "{mohm} mΩ read as {found:.1}");
        assert!((r.start_v - 12.9).abs() < 0.05, "resting before the match: {}", r.start_v);
        assert!(r.low_v < r.start_v - mohm / 1000.0 * 150.0, "the low point is under a heavy load: {}", r.low_v);
        assert!(r.peak_a.unwrap() > 150.0);
        assert_eq!(r.brownouts, 0);
    }
    // A harder match draws more and sags further, but the battery is the same.
    let easy = reading_of(&synthetic(12.9, 20.0, 0.8, true, 4));
    let hard = reading_of(&synthetic(12.9, 20.0, 1.4, true, 4));
    assert!(hard.sag_v > easy.sag_v + 0.5, "{} against {}", hard.sag_v, easy.sag_v);
    assert!((hard.mohm.unwrap() - easy.mohm.unwrap()).abs() < 2.5, "{:?} against {:?}", hard.mohm, easy.mohm);
}

#[test]
fn without_current_it_is_scored_by_its_sag() {
    let tired = reading_of(&synthetic(12.6, 32.0, 1.0, false, 5));
    let fresh = reading_of(&synthetic(12.9, 15.0, 1.0, false, 5));
    assert_eq!(tired.mohm, None);
    assert_eq!(fresh.mohm, None);
    assert!(tired.sag_v > fresh.sag_v + 1.0, "{} against {}", tired.sag_v, fresh.sag_v);
    assert!(tired.score() < fresh.score());
    let h = health(&[("Q1", &fresh), ("Q2", &tired)]);
    assert_eq!(h.measure, Measure::Sag);
    assert_eq!(h.trend.len(), 2);
}

#[test]
fn a_brownout_is_counted_and_scored_down() {
    // 45 mΩ under a hard match: the volts fall under 6.8 at the shoves.
    let r = reading_of(&synthetic(12.4, 45.0, 1.5, true, 6));
    assert!(r.brownouts >= 1, "{r:?}");
    assert!(r.low_v < 6.8);
    assert!(r.score() <= 75.0);
}

#[test]
fn the_demo_logs_read_as_the_demo_event_tells_it() {
    let specs = crate::offline::synth::demo_event();
    let mut by_label = std::collections::HashMap::new();
    for spec in &specs {
        let bytes = crate::offline::synth::write(spec);
        let r = reading_of(&bytes);
        // The demo robot logs no current: its batteries are scored by sag.
        assert_eq!(r.mohm, None, "{}", spec.name);
        assert!((r.start_v - spec.rest_v).abs() < 0.1, "{}: started at {} for {}", spec.name, r.start_v, spec.rest_v);
        // The low point is the summary's, which counts the whole log.
        let summary = crate::wpilog::summarize(&bytes[..]).unwrap();
        assert!((r.low_v - summary.battery_low.unwrap()).abs() < 0.01, "{}", spec.name);
        assert_eq!(r.brownouts, summary.brownouts, "{}", spec.name);
        by_label.insert(crate::offline::facts::label_from_name(&spec.name).unwrap(), r);
    }
    // Q14 browned out; the matches before it did not.
    assert_eq!(by_label["Q14"].brownouts, 1);
    assert!(by_label["Q14"].low_v < 6.8);
    assert!(["Q1", "Q3", "Q5", "Q7", "Q9", "Q12"].iter().all(|l| by_label[*l].brownouts == 0));
    // It sagged further than the quiet first match.
    assert!(by_label["Q14"].sag_v > by_label["Q1"].sag_v);
}

#[test]
fn a_log_with_no_battery_has_no_reading() {
    let mut bytes = b"WPILOG".to_vec();
    bytes.extend_from_slice(&0x0100u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(analyse(&bytes[..]).unwrap(), None);
    assert!(analyse(&b"not a log"[..]).is_err());
}

fn steady(mohm: f64) -> Reading {
    Reading { start_v: 12.8, low_v: 10.5, sag_v: 1.4, mohm: Some(mohm), brownouts: 0, peak_a: Some(180.0) }
}

#[test]
fn retire_soon_when_the_resistance_climbs_or_it_browns_out() {
    let (a, b, c) = (steady(18.0), steady(18.5), steady(19.0));
    assert_eq!(health(&[("Q1", &a), ("Q2", &b), ("Q3", &c)]).retire, None, "steady");
    let (d, e, f) = (steady(22.0), steady(24.0), steady(25.0));
    let h = health(&[("Q1", &a), ("Q2", &b), ("Q3", &c), ("Q4", &d), ("Q5", &e), ("Q6", &f)]);
    assert_eq!(h.measure, Measure::Resistance);
    assert_eq!(h.trend, vec![18.0, 18.5, 19.0, 22.0, 24.0, 25.0]);
    assert!(h.retire.as_deref().unwrap().starts_with("resistance up"), "{:?}", h.retire);
    // Past what any battery should read, whatever its past.
    let worn = steady(31.0);
    assert_eq!(health(&[("Q1", &worn)]).retire.as_deref(), Some("31 mOhm"));
    // A brownout in its last three matches.
    let browned = Reading { brownouts: 1, ..steady(19.0) };
    assert_eq!(health(&[("Q1", &a), ("Q9", &browned), ("Q10", &b)]).retire.as_deref(), Some("browned out in Q9"));
    // Four matches on, it is forgiven.
    assert_eq!(health(&[("Q9", &browned), ("Q10", &a), ("Q11", &b), ("Q12", &c)]).retire, None);
    // By sag where no current was logged.
    let sag = |v: f64| Reading { mohm: None, sag_v: v, ..steady(0.0) };
    let (s1, s2, s3) = (sag(1.4), sag(1.6), sag(2.2));
    assert_eq!(health(&[("Q1", &s1), ("Q2", &s2), ("Q3", &s3)]).retire.as_deref(), Some("sag up 0.8 v"));
}

// ----------------------------------------------------------------- state ---

fn fleet_of(names: &[&str]) -> Fleet {
    let mut f = Fleet::default();
    for n in names {
        f.add(n, Some(2025), NOW - 86_400).unwrap();
    }
    f
}

#[test]
fn a_battery_goes_round_charged_used_charging_resting_charged() {
    let mut f = fleet_of(&["7"]);
    let b = |f: &Fleet| f.get("7").unwrap().clone();
    assert_eq!(b(&f).state(NOW), State::Charged, "a new battery is taken as charged");
    assert_eq!(f.assign("7", "Q31", NOW).unwrap(), None);
    assert_eq!(b(&f).state(NOW), State::Used);
    assert_eq!(f.battery_for("q31"), Some("7"));
    f.put_on("7", NOW + 600).unwrap();
    assert_eq!(b(&f).state(NOW + 600), State::Charging { full: false });
    assert_eq!(b(&f).charging_for(NOW + 600 + 300), Some(300));
    assert!(f.put_on("7", NOW + 700).is_err(), "on already");
    assert_eq!(b(&f).state(NOW + 600 + CHARGE_SECONDS), State::Charging { full: true });
    let off = NOW + 600 + CHARGE_SECONDS;
    f.take_off("7", off).unwrap();
    assert!(f.take_off("7", off).is_err(), "off already");
    assert_eq!(b(&f).state(off), State::Resting);
    assert_eq!(b(&f).rest_left(off + 60), Some(REST_SECONDS - 60));
    assert_eq!(b(&f).state(off + REST_SECONDS - 1), State::Resting);
    assert_eq!(b(&f).state(off + REST_SECONDS), State::Charged);
    assert_eq!(b(&f).rest_left(off + REST_SECONDS), None);
    assert!(f.put_on("nine", NOW).is_err());
}

#[test]
fn picking_another_for_the_same_match_puts_the_first_back() {
    let mut f = fleet_of(&["1", "2"]);
    f.put_on("2", NOW - 600).unwrap();
    f.assign("1", "Q31", NOW).unwrap();
    // Still charging: it goes in, with a word about it.
    let caution = f.assign("2", "Q31", NOW + 30).unwrap();
    assert_eq!(caution.as_deref(), Some("2 was still charging"));
    assert_eq!(f.get("1").unwrap().state(NOW + 30), State::Charged, "back where it was");
    assert_eq!(f.battery_for("Q31"), Some("2"));
    assert_eq!(f.uses.len(), 1);
    // The same one again changes nothing.
    assert_eq!(f.assign("2", "Q31", NOW + 60).unwrap(), None);
    assert_eq!(f.uses.len(), 1);
    // Practice is never one match: each pick is its own.
    f.assign("1", PRACTICE, NOW + 90).unwrap();
    f.assign("1", PRACTICE, NOW + 120).unwrap();
    assert_eq!(f.uses.len(), 3);
}

#[test]
fn names_years_and_ids() {
    let mut f = Fleet::default();
    assert_eq!(f.add("Old Blue", Some(2022), NOW).unwrap(), "old-blue");
    assert_eq!(f.add("old  blue!", None, NOW).unwrap(), "old-blue-2", "a key of its own");
    assert!(f.add("Old Blue", Some(2022), NOW).is_err(), "the same battery twice");
    assert!(f.add("  ", None, NOW).is_err());
    assert!(f.add("—", None, NOW).is_err());
    assert!(f.add("a name far too long", None, NOW).is_err());
    assert_eq!(f.get("old-blue").unwrap().title(), "Old Blue · 2022");
    f.assign("old-blue", "Q1", NOW).unwrap();
    assert!(f.remove("old-blue").is_some());
    assert!(f.uses.is_empty(), "its matches go with it");
}

#[test]
fn rest_and_charge_times_read_plainly() {
    assert_eq!(minutes(6 * 60), "6 min");
    assert_eq!(minutes(6 * 60 - 20), "6 min", "a part minute counts");
    assert_eq!(minutes(65 * 60), "1 h 05");
    assert_eq!(short(32 * 60), "32m");
    assert_eq!(short(65 * 60), "1h05");
    assert_eq!(short(0), "0m");
}

// ------------------------------------------------------------- the pick ---

fn read_as(f: &mut Fleet, id: &str, label: &str, r: Reading) {
    f.uses.push(Use { battery: id.into(), label: label.into(), at: NOW - 7200, before: Charge::Off { since: NOW - 9000 }, log: Some(format!("{label}.wpilog")), reading: Some(r) });
}

#[test]
fn the_next_battery_is_the_charged_rested_one_in_the_best_health() {
    let mut f = fleet_of(&["1", "2", "3", "4", "5"]);
    read_as(&mut f, "1", "Q1", steady(24.0));
    read_as(&mut f, "2", "Q2", steady(16.0));
    read_as(&mut f, "3", "Q3", steady(14.0));
    read_as(&mut f, "4", "Q4", steady(13.0));
    // 3 is the healthiest of those charged, but resting; 4 is better still, but on the charger.
    f.put_on("3", NOW - 4000).unwrap();
    f.take_off("3", NOW - 300).unwrap();
    f.put_on("4", NOW - 600).unwrap();
    assert_eq!(f.next(NOW).unwrap().id, "2");
    // 5 is new (no matches): it comes after those known healthy.
    let order: Vec<&str> = f.ranked(NOW).into_iter().map(|i| f.batteries[i].id.as_str()).collect();
    assert_eq!(order, ["2", "1", "5", "3", "4"]);
    // Flagged to retire, it waits for the others.
    let browned = Reading { brownouts: 1, ..steady(15.0) };
    read_as(&mut f, "2", "Q8", browned);
    assert_eq!(f.next(NOW).unwrap().id, "1");
    // Used, none of them: the resting one nearest ready.
    for id in ["1", "2", "5"] {
        f.assign(id, &format!("Q{id}0"), NOW).unwrap();
    }
    assert_eq!(f.next(NOW).unwrap().id, "3");
    f.assign("3", "Q30", NOW).unwrap();
    assert_eq!(f.next(NOW).unwrap().id, "4", "on the charger, the last one left");
    f.assign("4", "Q60", NOW).unwrap();
    assert_eq!(f.next(NOW), None, "all used");
    // Among charged ones of equal health, the one used least lately.
    let mut g = fleet_of(&["a", "b"]);
    g.assign("a", "Q1", NOW - 9000).unwrap();
    g.put_on("a", NOW - 8000).unwrap();
    g.take_off("a", NOW - 4000).unwrap();
    assert_eq!(g.next(NOW).unwrap().id, "b");
}

#[test]
fn sorted_by_health_ready_or_name() {
    let mut f = fleet_of(&["10", "2", "Spare"]);
    read_as(&mut f, "10", "Q1", steady(14.0));
    read_as(&mut f, "2", "Q1", steady(26.0));
    let names = |order: Vec<usize>| order.into_iter().map(|i| f.batteries[i].name.clone()).collect::<Vec<_>>();
    assert_eq!(names(f.sorted(Sort::Name, NOW)), ["2", "10", "Spare"]);
    assert_eq!(names(f.sorted(Sort::Health, NOW)), ["10", "2", "Spare"]);
    assert_eq!(Sort::Health.next().next().next(), Sort::Health);
}

// ---------------------------------------------------- the island, the logs ---

fn view(now: i64, queue_in: Option<i64>, match_in: i64) -> crate::schedule::View {
    crate::schedule::View {
        team: 5805,
        matches: vec![crate::schedule::Match { key: "q31".into(), label: "Q31".into(), red: vec![5805], time: Some(now + match_in), queue_at: queue_in.map(|q| now + q), ..Default::default() }],
        ..Default::default()
    }
}

#[test]
fn the_island_asks_once_as_the_queue_nears() {
    with(|f| *f = fleet_of(&["1", "2"]));
    assert!(ask(&view(NOW, Some(15 * 60), 30 * 60), NOW).is_none(), "fifteen minutes off: not yet");
    let n = ask(&view(NOW, Some(8 * 60), 23 * 60), NOW).expect("asked");
    assert_eq!(n.title, "put 1 in for Q31?");
    assert_eq!(n.body, "charged and rested, the healthiest ready");
    assert_eq!(n.action.as_deref(), Some("builtin:Batteries"));
    assert_eq!(n.said(), "put 1 in for Q31?", "short enough for the island at every size");
    assert!(ask(&view(NOW, Some(8 * 60), 23 * 60), NOW).is_none(), "once a match");
    // The app opens on the pick until one is picked.
    assert_eq!(waiting(Some(&view(NOW, Some(8 * 60), 23 * 60)), NOW).as_deref(), Some("Q31"));
    with(|f| f.assign("2", "Q31", NOW)).unwrap();
    assert_eq!(waiting(Some(&view(NOW, Some(8 * 60), 23 * 60)), NOW), None);
    // With no queue time, twenty minutes before the match.
    with(|f| *f = fleet_of(&["1"]));
    assert!(ask(&view(NOW, None, 25 * 60), NOW).is_none());
    assert!(ask(&view(NOW, None, 19 * 60), NOW).is_some());
    // No fleet, no question.
    with(|f| *f = Fleet::default());
    assert!(ask(&view(NOW, Some(60), 900), NOW).is_none());
}

#[test]
fn each_match_log_goes_to_the_battery_that_was_in_for_it() {
    let mut f = fleet_of(&["1", "2"]);
    f.assign("1", "Q12", NOW).unwrap();
    f.assign("2", "Q14", NOW).unwrap();
    f.assign("1", PRACTICE, NOW).unwrap();
    let logs = ["FRC_20261116_101500_NYRO_Q14.wpilog".to_owned(), "FRC_20261116_095800_NYRO_Q12.wpilog".to_owned(), "FRC_20261115_171800.wpilog".to_owned()];
    assert_eq!(f.logs_wanted(&logs), vec![(0, logs[1].clone()), (1, logs[0].clone())]);
    // Read, it is not wanted again; and a newly flagged battery is told.
    with(|s| *s = f.clone());
    let bytes = crate::offline::synth::write(&crate::offline::synth::demo_event()[0]);
    let told = record(1, &logs[0], analyse(&bytes[..]).unwrap()).expect("browned out in Q14");
    assert_eq!(told.title, "retire 2 soon");
    assert_eq!(told.body, "browned out in Q14 · after Q14");
    assert!(record(0, &logs[1], Some(steady(18.0))).is_none(), "nothing to tell");
    assert_eq!(read(|f| f.logs_wanted(&logs)), vec![]);
    assert_eq!(read(|f| f.health("2").brownouts), 1);
}

#[test]
fn the_charger_sets_a_reminder_and_taking_it_off_clears_it() {
    with(|f| *f = fleet_of(&["7"]));
    assert_eq!(charger("7"), "7 on the charger");
    let id = read(|f| f.get("7").unwrap().reminder).expect("a reminder");
    let r = crate::reminders::with(|s| s.book.get(id).cloned()).expect("in Reminders");
    assert_eq!(r.text, "battery 7 charged: take it off");
    assert!(matches!(r.when, crate::reminders::When::In(at) if (at - now() - CHARGE_SECONDS).abs() < 5));
    assert!(charger("7").starts_with("7 off · rested in 15 min"));
    assert!(crate::reminders::with(|s| s.book.get(id).is_none()), "gone once it is off");
    assert_eq!(read(|f| f.get("7").unwrap().reminder), None);
    // Picked straight off the charger, its reminder goes too.
    charger("7");
    let id = read(|f| f.get("7").unwrap().reminder).unwrap();
    assert!(pick("7", "Q31").starts_with("7 in for Q31"));
    assert!(crate::reminders::with(|s| s.book.get(id).is_none()));
}

#[test]
fn kept_and_read_back_the_same() {
    let mut f = fleet_of(&["1", "Old Blue"]);
    f.put_on("1", NOW).unwrap();
    f.assign("old-blue", "Q3", NOW).unwrap();
    read_as(&mut f, "old-blue", "Q1", steady(19.5));
    f.asked.push("Q3".into());
    f.sort = Some(Sort::Name);
    f.get_mut("1").unwrap().reminder = Some(4);
    let text = serde_json::to_string(&f.to_json()).unwrap();
    assert_eq!(Fleet::from_json(&serde_json::from_str(&text).unwrap()), f);
}

#[test]
fn a_scanned_label_opens_its_battery() {
    with(|f| *f = fleet_of(&["7"]));
    assert!(scanned("catalyst-battery:7").is_some());
    assert!(scanned("catalyst-battery:8").is_none());
    assert!(scanned("https://example.com").is_none());
}

// ---------------------------------------------------------------- labels ---

#[test]
fn the_label_sheet_prints_at_size_and_its_codes_read_back() {
    let mut f = fleet_of(&["1", "2", "Old Blue", "12"]);
    f.batteries[3].year = None;
    let mut png = Vec::new();
    label::write(&f.batteries, Some(5805), &mut png).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(&png));
    let mut reader = decoder.read_info().unwrap();
    let info = reader.info().clone();
    assert_eq!((info.width, info.height), (label::SHEET_W as u32, label::SHEET_H as u32));
    assert_eq!(info.color_type, png::ColorType::Grayscale);
    let dims = info.pixel_dims.unwrap();
    assert!((dims.xppu as f64 * 0.0254 - 300.0).abs() < 0.5, "300 dpi");
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut buf).unwrap();
    for (i, b) in f.batteries.iter().enumerate() {
        let r = label::label_rect(i);
        let (x0, y0, w, h) = (r.x as usize, r.y as usize, r.w as usize, r.h as usize);
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| buf[(y0 + y) * label::SHEET_W + x0 + x]);
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1, "one code on {}'s label", b.name);
        assert_eq!(grids[0].decode().unwrap().1, format!("catalyst-battery:{}", b.id));
    }
    // Beyond the labels the sheet is blank.
    let last = label::label_rect(f.batteries.len());
    assert!(buf[(last.y as usize + 150) * label::SHEET_W + last.x as usize + 150] == 255);
}

// ------------------------------------------------------------- the pages ---

#[test]
fn every_page_draws_whole_at_each_size_with_its_icons() {
    use crate::gfx::{Gfx, TextSize};
    use crate::shell::Screen;
    let robot = crate::robot::Robot::demo();
    with(|f| *f = super::shots::demo_fleet(now()));
    for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
        for tokens in [bezel::Tokens::dark(), bezel::Tokens::light()] {
            let mut g = Gfx::new(tokens);
            g.set_text_size(size);
            let mut pages: Vec<Box<dyn Screen>> = vec![
                Box::new(app::Fleet::new()),
                Box::new(app::Detail::new("6")),
                Box::new(app::Detail::new("old-blue")),
                Box::new(app::Pick::new("Q31", &robot, true)),
                Box::new(app::Add::new()),
            ];
            for page in &mut pages {
                page.draw(&mut g, &robot);
            }
            assert!(g.missing_icons().is_empty(), "{:?}", g.missing_icons());
            // Each card's words whole on a card at this size.
            let cols = if size == TextSize::Large { 2.0 } else { 3.0 };
            let card_w = (crate::gfx::W - 2.0 * crate::gfx::PAD - 5.0 - (cols - 1.0) * crate::gfx::GAP) / cols;
            let inside = card_w - 2.0 * app::CARD_IN;
            let icon = g.ty.name.size - 3.0;
            for words in ["full in 60m", "resting 15m", "used · q22", "take it off"] {
                assert!(g.measure(words, g.ty.caption) <= inside, "{words} at {size:?}");
            }
            // A name of eight letters beside the state's icon.
            assert!(g.measure("Old Blue", g.ty.name) <= inside - icon - 4.0, "the name at {size:?}");
            // And room for the trend beside the health.
            for health in ["31 mOhm", "−2.4 v"] {
                assert!(inside - g.measure(health, g.ty.label) - app::SPARK_GAP >= app::SPARK_LEAST, "{health} at {size:?}");
            }
            // The rows of a battery's matches and of the pick, whole in
            // either theme (Bezel's names are mono, its values Sans).
            for bezel in [false, true] {
                let (name_style, value_style) = if bezel { (g.ty.label, g.ty.body) } else { (g.ty.body, g.ty.label) };
                for (name, value) in [
                    ("    q14 · brownout", "12.5–4.5 v"),
                    ("    q12", "12.6–6.9 v · 28 mOhm"),
                    ("    q9", "12.5–7.4 v · sag 3.9"),
                    ("    retire soon", "resistance up 31%"),
                    ("    practice", "no log read"),
                    ("    Old Blue · charging", "−3.9 v"),
                    ("    Old Blue · resting", "next"),
                    ("    Old Blue", "next · 31 mOhm"),
                ] {
                    let name = if bezel { name.to_lowercase() } else { name.to_owned() };
                    let w = g.measure(&name, name_style) + g.measure(value, value_style) + 30.0;
                    assert!(w <= crate::gfx::W - 2.0 * crate::gfx::PAD - 5.0, "{name} | {value} at {size:?}, bezel {bezel}: {w:.0}");
                }
            }
        }
    }
}

#[test]
fn the_shell_asks_when_the_demo_queue_nears() {
    testing::watch(true);
    with(|f| *f = fleet_of(&["1", "2"]));
    let mut shell = crate::shell::Shell::new(crate::robot::Robot::demo());
    for _ in 0..30 {
        if shell.tick(1.0 / 60.0) {
            shell.draw();
        }
    }
    testing::watch(false);
    assert!(read(|f| f.asked.contains(&"Q31".to_owned())), "the demo's Q31 queues in eight minutes");
}
