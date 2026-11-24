//! Photo documentation's tests: fingerprints and named places (retroactive
//! too), the event and match, BeaconDB and the gazetteer, the caption queue
//! against a stand-in for Anthropic (offline, retries, the limit, captions
//! off sending nothing), the sidecar's round trip, the log and its export.
//!
//! No test reaches a real service: Anthropic and BeaconDB are stand-ins on
//! this computer, answering with replies shaped as their documentation
//! shows them (`recorded`). The one real caption (`a_real_caption`) is
//! ignored unless asked for, and asks only with `ANTHROPIC_API_KEY` set.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::caption::{self, Api, Conditions, Did};
use super::event;
use super::locate;
use super::log::{self, Range};
use super::meta::{self, About};
use super::places::{self, Ap, Places};
use super::store::{Job, Ledger, Record};
use crate::schedule::{Event, Match, View};

fn scratch(name: &str) -> PathBuf {
    crate::shell::tests::sandbox();
    let dir = std::env::temp_dir().join(format!("catalyst-photolog-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("photos")).unwrap();
    std::fs::create_dir_all(dir.join("private")).unwrap();
    dir
}

fn ledger(dir: &Path) -> Mutex<Ledger> {
    Mutex::new(Ledger::open(&dir.join("private"), &dir.join("photos")))
}

#[cfg(not(target_arch = "aarch64"))]
fn photo(dir: &Path, name: &str, w: usize, h: usize) -> PathBuf {
    let path = dir.join("photos").join(name);
    crate::camera::save_jpeg(&path, &crate::camera::scene(w, h, 3.0, false)).unwrap();
    path
}

fn ap(bssid: u64, dbm: i8) -> Ap {
    Ap { bssid, dbm, share: true }
}

// ------------------------------------------------------------ fingerprints ---

/// A tiny random source: the same every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 10_000) as f32 / 10_000.0
    }
}

/// A place's access points: (address, signal there).
fn site(base: u64, aps: &[i8]) -> Vec<(u64, i8)> {
    aps.iter().enumerate().map(|(k, dbm)| (base + (k as u64) * 0x100, *dbm)).collect()
}

/// A scan at a site: each signal off by up to ±8 dB, the weak ones (under
/// −85) seen only now and then, an access point or two of passers-by.
fn scan(rng: &mut Rng, aps: &[(u64, i8)]) -> Vec<Ap> {
    scan_by(rng, aps, 8.0)
}

/// The same, each signal off by up to `spread` dB.
fn scan_by(rng: &mut Rng, aps: &[(u64, i8)], spread: f32) -> Vec<Ap> {
    let mut out = Vec::new();
    for &(b, dbm) in aps {
        let d = f32::from(dbm) + (rng.next() - 0.5) * 2.0 * spread;
        if d < -85.0 && rng.next() < 0.5 {
            continue;
        }
        out.push(ap(b, d.clamp(-100.0, -20.0) as i8));
    }
    if rng.next() < 0.5 {
        out.push(ap(0x00_ee_00_00_00_00 + (rng.next() * 1e6) as u64 * 16, -82));
    }
    places::strongest(out)
}

#[test]
fn the_threshold_tells_the_same_room_from_the_one_next_door_and_across_town() {
    let mut rng = Rng(0x5805);
    // The shop: its own network strong, the school's weaker.
    let shop = [site(0x10_00_00_00_00_00, &[-45, -52, -60, -66, -71, -79, -84, -88]), site(0x20_00_00_00_00_00, &[-75, -80, -86])].concat();
    // The classroom next door: the school's network strong, the shop's weak.
    let next_door = [site(0x10_00_00_00_00_00, &[-80, -84, -88, -90]), site(0x20_00_00_00_00_00, &[-48, -55, -62]), site(0x30_00_00_00_00_00, &[-58, -70])].concat();
    let across_town = site(0x40_00_00_00_00_00, &[-50, -58, -63, -70, -77, -82]);
    let mut same = Vec::new();
    let mut near = Vec::new();
    let mut far = Vec::new();
    // Another day in the shop: the signals rougher (±12 dB), its strongest
    // router switched off.
    let mut other_day = Vec::new();
    let without_main: Vec<(u64, i8)> = shop[1..].to_vec();
    for _ in 0..300 {
        let a = scan(&mut rng, &shop);
        same.push(places::likeness(&a, &scan(&mut rng, &shop)).0);
        other_day.push(places::likeness(&a, &scan_by(&mut rng, &without_main, 12.0)).0);
        near.push(places::likeness(&a, &scan(&mut rng, &next_door)).0);
        far.push(places::likeness(&a, &scan(&mut rng, &across_town)).0);
    }
    let lowest = |v: &[f32]| v.iter().cloned().fold(1.0, f32::min);
    let highest = |v: &[f32]| v.iter().cloned().fold(0.0, f32::max);
    let share = |v: &[f32], f: &dyn Fn(f32) -> bool| v.iter().filter(|s| f(**s)).count() as f32 / v.len() as f32;
    let pct = |v: &[f32], p: f32| {
        let mut s = v.to_vec();
        s.sort_by(f32::total_cmp);
        s[((s.len() - 1) as f32 * p) as usize]
    };
    eprintln!("same room: lowest {:.3}, 1% {:.3}, 5% {:.3}, median {:.3}", lowest(&same), pct(&same, 0.01), pct(&same, 0.05), pct(&same, 0.5));
    eprintln!("another day: lowest {:.3}, 1% {:.3}, 5% {:.3}, median {:.3}", lowest(&other_day), pct(&other_day, 0.01), pct(&other_day, 0.05), pct(&other_day, 0.5));
    eprintln!("next door: median {:.3}, 95% {:.3}, 99% {:.3}, highest {:.3}", pct(&near, 0.5), pct(&near, 0.95), pct(&near, 0.99), highest(&near));
    // The same room is recognised every time, another day almost always;
    // next door, never.
    assert!(lowest(&same) >= places::SAME_PLACE, "same room: lowest {}", lowest(&same));
    assert!(share(&other_day, &|s| s >= places::SAME_PLACE) >= 0.95, "another day: {}", share(&other_day, &|s| s >= places::SAME_PLACE));
    assert!(highest(&near) < places::SAME_PLACE, "next door reached {}", highest(&near));
    assert_eq!(highest(&far), 0.0);
}

#[test]
fn an_access_points_other_radios_count_as_one_and_what_travels_is_never_kept() {
    // The same access point's 2.4 and 5 GHz radios, and a guest network on it.
    let a = vec![ap(0x7c_10_c9_4a_31_20, -50), ap(0x30_00_00_00_00_01, -70)];
    let b = vec![ap(0x7c_10_c9_4a_31_24, -52), ap(0x7e_10_c9_4a_31_20, -55), ap(0x30_00_00_00_00_01, -72)];
    let (score, common) = places::likeness(&a, &b);
    assert_eq!(common, 2);
    assert!(score > 0.8, "{score}");
    let text = "bssid / frequency / signal level / flags / ssid\n\
        7c:10:c9:4a:31:20\t2437\t-48\t[WPA2-PSK-CCMP][ESS]\tWorkshop\n\
        00:11:22:33:44:55\t5180\t-40\t[WPA2-PSK-CCMP][ESS]\t5805_Robot\n\
        00:11:22:33:44:66\t5180\t-41\t[WPA2-PSK-CCMP][ESS]\t5805\n\
        aa:bb:cc:dd:ee:01\t2412\t-60\t[WPA2-PSK-CCMP][ESS]\tDaniel's iPhone\n\
        3c:37:86:12:aa:01\t2412\t-63\t[WPA2-PSK-CCMP][ESS]\tCafe_nomap\n\
        a4:2b:b0:77:10:02\t2462\t-70\t[WPA2-PSK-CCMP][ESS]\t\n\
        a4:2b:b0:77:10:03\t2462\t-71\t[WPA2-PSK-CCMP][ESS]\tSchool\n";
    let fp = places::parse_scan(text);
    let kept: Vec<String> = fp.iter().map(|a| places::bssid_text(a.bssid)).collect();
    assert_eq!(kept, ["7c:10:c9:4a:31:20", "3c:37:86:12:aa:01", "a4:2b:b0:77:10:02", "a4:2b:b0:77:10:03"], "the robot's radio and the phone are left out");
    let share: Vec<bool> = fp.iter().map(|a| a.share).collect();
    assert_eq!(share, [true, false, false, true], "a _nomap network and a hidden one are never sent anywhere");
    assert!(places::travels("5805_Robot") && places::travels("254") && !places::travels("Workshop") && !places::travels("2.4GHz Lab"));
}

#[test]
fn a_named_place_names_later_photos_and_earlier_ones_too() {
    let dir = scratch("naming");
    let l = ledger(&dir);
    let shop = places::demo_place("shop");
    let pit = places::demo_place("pit");
    // Three photos before anything is named: two in the shop, one in the pit.
    for (name, fp) in [("IMG_20261119_100000.jpg", &shop), ("IMG_20261119_110000.jpg", &shop), ("IMG_20261121_090000.jpg", &pit)] {
        std::fs::write(dir.join("photos").join(name), b"jpeg").unwrap();
        let mut moved: Vec<Ap> = fp.iter().map(|a| Ap { dbm: a.dbm - 3, ..*a }).collect();
        moved.truncate(4);
        l.lock().unwrap().note_taken(name, 1_763_000_000, moved, None);
    }
    assert_eq!(meta::read(&dir.join("photos/IMG_20261119_100000.jpg")).place, "", "nothing named yet");
    // Named from the second: the first takes the name too; the pit's does not.
    let id = l.lock().unwrap().name_place("IMG_20261119_110000.jpg", "  Shop ").unwrap();
    let place = |n: &str| meta::read(&dir.join("photos").join(n)).place;
    assert_eq!(place("IMG_20261119_100000.jpg"), "Shop");
    assert_eq!(place("IMG_20261119_110000.jpg"), "Shop");
    assert_eq!(place("IMG_20261121_090000.jpg"), "");
    assert_eq!(l.lock().unwrap().photos_of(id), 2);
    // A photo taken there afterwards is named as it is taken.
    std::fs::write(dir.join("photos/IMG_20261122_080000.jpg"), b"jpeg").unwrap();
    l.lock().unwrap().note_taken("IMG_20261122_080000.jpg", 1_763_100_000, shop.clone(), None);
    assert_eq!(place("IMG_20261122_080000.jpg"), "Shop");
    // The same name again is another look at it, not a second place.
    l.lock().unwrap().name_place("IMG_20261121_090000.jpg", "shop").unwrap();
    assert_eq!(l.lock().unwrap().places.list.len(), 1);
    assert_eq!(place("IMG_20261121_090000.jpg"), "Shop");
    // Renamed: every photo of it. Deleted: none keeps it.
    {
        let mut g = l.lock().unwrap();
        assert!(g.places.rename(id, "Workshop"));
        g.apply_places();
    }
    assert_eq!(place("IMG_20261119_100000.jpg"), "Workshop");
    assert_eq!(place("IMG_20261121_090000.jpg"), "Workshop");
    {
        let mut g = l.lock().unwrap();
        g.places.delete(id);
        assert_eq!(g.apply_places(), 4);
    }
    assert_eq!(place("IMG_20261122_080000.jpg"), "");
    // Kept, and read back the same.
    l.lock().unwrap().places.name("Pit", &pit);
    l.lock().unwrap().save();
    let again = Ledger::open(&dir.join("private"), &dir.join("photos"));
    assert_eq!(again.places, l.lock().unwrap().places);
    assert_eq!(again.records, l.lock().unwrap().records);
    // A photo with no Wi-Fi cannot name a place.
    l.lock().unwrap().note_taken("IMG_20261123_080000.jpg", 1, Vec::new(), None);
    assert!(l.lock().unwrap().name_place("IMG_20261123_080000.jpg", "Home").is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn names_are_tidied_and_a_rename_cannot_take_anothers() {
    let mut p = Places::default();
    let a = p.name(" Pit \n crew ", &places::demo_place("pit")).unwrap();
    let b = p.name("Shop", &places::demo_place("shop")).unwrap();
    assert_eq!(p.get(a).unwrap().name, "Pit crew");
    assert!(!p.rename(b, "pit CREW"), "two places, one name");
    assert!(p.name("", &places::demo_place("home")).is_none());
    assert_eq!(places::clean_name(&"x".repeat(40)).len(), places::NAME_MOST);
}

// ------------------------------------------------------------- the event ---

fn view() -> View {
    let m = |label: &str, time: i64| Match { key: label.to_lowercase(), label: label.into(), red: vec![5805], blue: vec![254], time: Some(time), ..Match::default() };
    View {
        team: 5805,
        event: Some(Event { key: "2026nyro".into(), name: "Finger Lakes Regional".into(), start: "2026-03-12".into(), end: "2026-03-15".into(), city: "Rochester, NY".into() }),
        matches: vec![m("Q31", 10_000), m("Q32", 13_000), m("Q40", 30_000)],
        ..View::default()
    }
}

#[test]
fn a_photo_is_tagged_with_the_event_and_where_it_fell_among_the_matches() {
    let v = view();
    let tag = |at: i64, day: &str| event::tag(&v, at, day).map(|t| t.context);
    // A match's own time, and a minute before to five after: during it.
    assert_eq!(tag(10_000, "2026-03-13").as_deref(), Some("during Q31"));
    assert_eq!(tag(9_950, "2026-03-13").as_deref(), Some("during Q31"));
    assert_eq!(tag(10_290, "2026-03-13").as_deref(), Some("during Q31"));
    // Between matches: before the next within an hour and a half.
    assert_eq!(tag(11_000, "2026-03-13").as_deref(), Some("before Q32"));
    // Else after the last within the hour.
    assert_eq!(tag(13_000 + 1_200, "2026-03-13").as_deref(), Some("after Q32"));
    // A quiet stretch: the event, no match.
    assert_eq!(tag(20_000, "2026-03-13").as_deref(), Some(""));
    let t = event::tag(&v, 10_000, "2026-03-13").unwrap();
    assert_eq!((t.event.as_str(), t.venue.as_str(), t.key.as_str()), ("Finger Lakes Regional", "Rochester, NY", "2026nyro"));
    // Days away from the event, and no match near: not at it.
    assert_eq!(event::tag(&v, 500_000, "2026-04-02"), None);
    // On the event's days, however far from a match.
    assert!(event::tag(&v, 900_000, "2026-03-15").is_some());
    // No event known: nothing.
    assert_eq!(event::tag(&View::default(), 10_000, "2026-03-13"), None);
}

#[test]
fn the_schedule_says_where_its_event_is_and_keeps_it() {
    let events = json!([{"key": "2026nyro", "name": "Finger Lakes Regional", "start_date": "2026-03-12", "end_date": "2026-03-15", "city": "Rochester", "state_prov": "NY", "country": "USA"}]);
    let e = crate::schedule::pick_event(&events, "2026-03-13").unwrap();
    assert_eq!(e.city, "Rochester, NY");
}

#[test]
fn a_moment_noted_tags_its_sidecar_with_the_place_and_the_match() {
    let dir = scratch("moment");
    let l = ledger(&dir);
    l.lock().unwrap().places.name("Pit", &places::demo_place("pit"));
    let tag = event::tag(&view(), 13_000, "2026-03-13");
    l.lock().unwrap().note_taken("IMG_20260313_093000.jpg", 13_000, places::demo_place("pit"), tag.as_ref());
    let about = meta::read(&dir.join("photos/IMG_20260313_093000.jpg"));
    assert_eq!((about.place.as_str(), about.event.as_str(), about.context.as_str()), ("Pit", "Finger Lakes Regional", "during Q32"));
    assert_eq!(about.place_line().as_deref(), Some("Pit"), "a named place first");
    let at_event = About { event: "Finger Lakes Regional".into(), venue: "Rochester, NY".into(), town: "Henrietta, NY".into(), ..About::default() };
    assert_eq!(at_event.place_line().as_deref(), Some("Finger Lakes Regional · Rochester, NY"), "else the event's venue");
    assert_eq!(About { town: "Henrietta, NY".into(), ..About::default() }.place_line().as_deref(), Some("Henrietta, NY"), "else the town");
    std::fs::remove_dir_all(&dir).ok();
}

// --------------------------------------------------------- a stand-in server ---

/// What a stand-in server was asked: method, path, headers (lowercased
/// names), body.
#[derive(Clone, Debug)]
struct Asked {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Asked {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

type Answer = Box<dyn Fn(&Asked) -> (u16, String) + Send + Sync>;

/// A server on this computer answering each request with `answer`, keeping
/// what was asked.
fn stand_in(answer: Answer) -> (String, Arc<Mutex<Vec<Asked>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).ok();
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).map_or(true, |n| n <= 2) {
                    break;
                }
                if let Some((n, v)) = line.trim_end().split_once(':') {
                    headers.push((n.trim().to_lowercase(), v.trim().to_owned()));
                }
            }
            let length: usize = headers.iter().find(|(n, _)| n == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).ok();
            let mut parts = first.split_whitespace();
            let a = Asked { method: parts.next().unwrap_or("").into(), path: parts.next().unwrap_or("").into(), headers, body: String::from_utf8_lossy(&body).into_owned() };
            let (code, reply) = answer(&a);
            seen.lock().unwrap().push(a);
            let mut stream = stream;
            write!(stream, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).ok();
        }
    });
    (base, asked)
}

/// Replies as Anthropic's documentation shows them (Message Batches:
/// create, retrieve, results as JSON lines; Messages; errors).
mod recorded {
    use serde_json::{json, Value};

    pub fn batch(id: &str, status: &str) -> String {
        json!({
            "id": id, "type": "message_batch", "processing_status": status,
            "request_counts": {"processing": if status == "ended" { 0 } else { 2 }, "succeeded": if status == "ended" { 2 } else { 0 }, "errored": 0, "canceled": 0, "expired": 0},
            "ended_at": null, "created_at": "2026-11-21T18:37:24.100435Z", "expires_at": "2026-11-22T18:37:24.100435Z",
            "cancel_initiated_at": null, "results_url": if status == "ended" { json!(format!("https://api.anthropic.com/v1/messages/batches/{id}/results")) } else { Value::Null }
        })
        .to_string()
    }

    pub fn message(text: &str, input: u64, output: u64) -> Value {
        json!({
            "id": "msg_014VwiXbi91y3JMjcpyGBHX5", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "content": [{"type": "thinking", "thinking": "", "signature": "EqQB"}, {"type": "text", "text": text}],
            "stop_reason": "end_turn", "stop_sequence": null,
            "usage": {"input_tokens": input, "output_tokens": output, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}
        })
    }

    pub fn succeeded(id: &str, text: &str) -> String {
        json!({"custom_id": id, "result": {"type": "succeeded", "message": message(text, 1311, 104)}}).to_string()
    }

    pub fn errored(id: &str, kind: &str) -> String {
        json!({"custom_id": id, "result": {"type": "errored", "error": {"type": "error", "error": {"type": kind, "message": "messages.0.content.0.image.source.data: image too large"}}}}).to_string()
    }

    pub fn refused(id: &str) -> String {
        let mut m = message("", 1311, 12);
        m["stop_reason"] = json!("refusal");
        m["content"] = json!([]);
        json!({"custom_id": id, "result": {"type": "succeeded", "message": m}}).to_string()
    }

    pub fn error(kind: &str, message: &str) -> String {
        json!({"type": "error", "error": {"type": kind, "message": message}, "request_id": "req_011CSHoEeqs5C35K2UUqR7Fy"}).to_string()
    }
}

/// An album of photos waiting for captions, noted `ago` seconds before `now`.
#[cfg(not(target_arch = "aarch64"))]
fn waiting(dir: &Path, l: &Mutex<Ledger>, names: &[&str], at: i64) {
    l.lock().unwrap().prefs.captions = true;
    for n in names {
        photo(dir, n, 1640, 1232);
        l.lock().unwrap().note_taken(n, at, places::demo_place("shop"), None);
    }
}

const KEY: &str = "sk-ant-api03-test-key-not-real";

#[cfg(not(target_arch = "aarch64"))]
#[test]
fn captions_go_in_a_batch_come_back_by_their_ids_and_the_batch_is_deleted() {
    let dir = scratch("batch");
    let l = ledger(&dir);
    let now = 1_795_000_000;
    waiting(&dir, &l, &["IMG_20261121_100000.jpg", "IMG_20261121_100500.jpg"], now - 600);
    let polls = Arc::new(Mutex::new(0));
    let p = polls.clone();
    let (base, asked) = stand_in(Box::new(move |a: &Asked| match (a.method.as_str(), a.path.as_str()) {
        ("POST", "/v1/messages/batches") => (200, recorded::batch("msgbatch_01HkcTjaV5uDC8jWR4ZsDV8d", "in_progress")),
        ("GET", "/v1/messages/batches/msgbatch_01HkcTjaV5uDC8jWR4ZsDV8d") => {
            *p.lock().unwrap() += 1;
            let ended = *p.lock().unwrap() > 1;
            (200, recorded::batch("msgbatch_01HkcTjaV5uDC8jWR4ZsDV8d", if ended { "ended" } else { "in_progress" }))
        }
        // In any order: the second first.
        ("GET", "/v1/messages/batches/msgbatch_01HkcTjaV5uDC8jWR4ZsDV8d/results") => {
            (200, [recorded::succeeded("IMG_20261121_100500-1", "\"Drilling the new intake plate on the shop's drill press.\""), recorded::succeeded("IMG_20261121_100000-0", "The swerve module with its cover off.")].join("\n"))
        }
        ("DELETE", "/v1/messages/batches/msgbatch_01HkcTjaV5uDC8jWR4ZsDV8d") => (200, json!({"id": "msgbatch_01HkcTjaV5uDC8jWR4ZsDV8d", "type": "message_batch_deleted"}).to_string()),
        _ => (404, recorded::error("not_found_error", "no such thing")),
    }));
    let api = Api { base, key: KEY.into(), demo: false, stamp: 0 };
    assert_eq!(caption::step(&l, Some(&api), now, Conditions::all()), Did::Sent(2));
    {
        let a = asked.lock().unwrap();
        let create = &a[0];
        // The key in its header, the API's version, nothing about the team.
        assert_eq!(create.header("x-api-key"), Some(KEY));
        assert_eq!(create.header("anthropic-version"), Some("2026-10-21"));
        assert_eq!(create.header("user-agent"), Some("CatalystLinux-Photos/1.4"));
        let body: Value = serde_json::from_str(&create.body).unwrap();
        let requests = body["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 2);
        let params = &requests[0]["params"];
        assert_eq!(requests[0]["custom_id"], "IMG_20261121_100000-0");
        assert_eq!(params["model"], "claude-opus-5-5");
        assert_eq!(params["output_config"]["effort"], "low");
        assert!(params.get("fallbacks").is_none(), "batches take no fallbacks");
        assert!(params.get("thinking").is_none(), "opus 5.5 thinks adaptively; it cannot be turned off");
        let content = params["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["source"]["media_type"], "image/jpeg");
        // The photo, shrunk to 1024 on its long side.
        let jpeg = speech::base64_decode(content[0]["source"]["data"].as_str().unwrap()).unwrap();
        let mut d = jpeg_decoder::Decoder::new(&jpeg[..]);
        d.read_info().unwrap();
        let info = d.info().unwrap();
        assert_eq!((info.width, info.height), (1024, 769));
        assert!(content[1]["text"].as_str().unwrap().contains("Shop") || content[1]["text"].as_str().unwrap().contains("Taken"));
        assert!(params["system"].as_str().unwrap().contains("Never guess"));
    }
    // Looked at: not done yet.
    let later = now + 61;
    assert_eq!(caption::step(&l, Some(&api), later, Conditions::all()), Did::Polled);
    // Looked at again once due: done, read, deleted.
    let due = l.lock().unwrap().batches[0].next_poll;
    assert_eq!(caption::step(&l, Some(&api), due, Conditions::all()), Did::Took(2));
    let methods: Vec<String> = asked.lock().unwrap().iter().map(|a| format!("{} {}", a.method, a.path)).collect();
    assert_eq!(methods.last().unwrap(), "DELETE /v1/messages/batches/msgbatch_01HkcTjaV5uDC8jWR4ZsDV8d");
    let caption = |n: &str| meta::read(&dir.join("photos").join(n));
    assert_eq!(caption("IMG_20261121_100000.jpg").caption, "The swerve module with its cover off.");
    assert_eq!(caption("IMG_20261121_100500.jpg").caption, "Drilling the new intake plate on the shop's drill press.", "quotes taken off");
    assert_eq!(caption("IMG_20261121_100500.jpg").by, "claude-opus-5-5");
    let g = l.lock().unwrap();
    assert!(g.batches.is_empty());
    assert!(g.records.values().all(|r| r.caption == Job::Done));
    // At half Opus 5.5's prices: (1311 × $4 + 104 × $20) / 2 per million.
    let each = (1311.0 * 4.0 + 104.0 * 20.0) / 2.0;
    assert_eq!(g.records["IMG_20261121_100000.jpg"].cost, each as u64);
    assert_eq!(g.spent(&super::store::month_of(due)), 2 * each as u64);
    drop(g);
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(not(target_arch = "aarch64"))]
#[test]
fn with_captions_off_nothing_is_sent_and_offline_they_wait() {
    let dir = scratch("off");
    let l = ledger(&dir);
    let now = 1_795_000_000;
    waiting(&dir, &l, &["IMG_20261121_100000.jpg"], now - 600);
    let (base, asked) = stand_in(Box::new(|_: &Asked| (500, String::new())));
    let api = Api { base, key: KEY.into(), demo: false, stamp: 0 };
    // Off: not a byte.
    l.lock().unwrap().prefs.captions = false;
    assert_eq!(caption::step(&l, Some(&api), now, Conditions::all()), Did::Waits("captions are off"));
    // On, but offline, on a low battery, or busy: it waits.
    l.lock().unwrap().prefs.captions = true;
    let offline = Conditions { online: false, ..Conditions::all() };
    assert_eq!(caption::step(&l, Some(&api), now, offline), Did::Waits("waiting to be online"));
    let low = Conditions { power: false, ..Conditions::all() };
    assert_eq!(caption::step(&l, Some(&api), now, low), Did::Waits("waiting for the charger or more battery"));
    // No key: it waits.
    assert_eq!(caption::step(&l, None, now, Conditions::all()), Did::Waits("no claude key"));
    // A photo too new waits for the burst to end.
    let fresh = dir.join("photos/IMG_20261121_110000.jpg");
    crate::camera::save_jpeg(&fresh, &crate::camera::scene(320, 240, 1.0, false)).unwrap();
    l.lock().unwrap().note_taken("IMG_20261121_110000.jpg", now - 10, Vec::new(), None);
    assert!(matches!(caption::step(&l, Some(&api), now, Conditions::all()), Did::Nothing(Some(_))));
    assert!(asked.lock().unwrap().is_empty(), "nothing was sent");
    // A photo taken while captions were off is never sent later.
    l.lock().unwrap().prefs.captions = false;
    l.lock().unwrap().note_taken("IMG_20261121_120000.jpg", now, Vec::new(), None);
    assert_eq!(l.lock().unwrap().records["IMG_20261121_120000.jpg"].caption, Job::None);
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(not(target_arch = "aarch64"))]
#[test]
fn failures_back_off_a_refused_key_waits_and_a_bad_photo_is_given_up() {
    let dir = scratch("retry");
    let l = ledger(&dir);
    let now = 1_795_000_000;
    waiting(&dir, &l, &["IMG_20261121_100000.jpg", "IMG_20261121_100100.jpg", "IMG_20261121_100200.jpg"], now - 600);
    let mode = Arc::new(Mutex::new(529u16));
    let m = mode.clone();
    let (base, asked) = stand_in(Box::new(move |a: &Asked| {
        let code = *m.lock().unwrap();
        match (code, a.method.as_str(), a.path.as_str()) {
            (529, ..) => (529, recorded::error("overloaded_error", "Overloaded")),
            (401, ..) => (401, recorded::error("authentication_error", "invalid x-api-key")),
            (_, "POST", "/v1/messages/batches") => (200, recorded::batch("msgbatch_2", "in_progress")),
            (_, "GET", "/v1/messages/batches/msgbatch_2") => (200, recorded::batch("msgbatch_2", "ended")),
            (_, "GET", "/v1/messages/batches/msgbatch_2/results") => (
                200,
                [recorded::errored("IMG_20261121_100000-0", "invalid_request_error"), recorded::errored("IMG_20261121_100100-1", "api_error"), recorded::refused("IMG_20261121_100200-2")].join("\n"),
            ),
            _ => (200, "{}".into()),
        }
    }));
    let api = Api { base, key: KEY.into(), demo: false, stamp: 0 };
    // Busy: everything waits, longer each time.
    assert!(matches!(caption::step(&l, Some(&api), now, Conditions::all()), Did::Failed(_)));
    let first = l.lock().unwrap().pause_until;
    assert_eq!(first, now + caption::backoff(1));
    assert!(matches!(caption::step(&l, Some(&api), now + 5, Conditions::all()), Did::Nothing(Some(t)) if t == first), "not asked again before then");
    assert!(matches!(caption::step(&l, Some(&api), first, Conditions::all()), Did::Failed(_)));
    assert!(l.lock().unwrap().pause_until - first > first - now, "backs off further");
    assert_eq!(asked.lock().unwrap().len(), 2);
    // The key refused: it waits for the keys to change, asking nothing.
    *mode.lock().unwrap() = 401;
    let t = l.lock().unwrap().pause_until;
    assert_eq!(caption::step(&l, Some(&api), t, Conditions::all()), Did::Waits("claude refused the key"));
    assert_eq!(caption::step(&l, Some(&api), t + 3600, Conditions::all()), Did::Waits("claude refused the key"));
    assert_eq!(asked.lock().unwrap().len(), 3);
    // The keys changed (a new time on the file): asked again.
    l.lock().unwrap().refused = Some(-1);
    *mode.lock().unwrap() = 200;
    assert_eq!(caption::step(&l, Some(&api), t + 3600, Conditions::all()), Did::Sent(3));
    let due = l.lock().unwrap().batches[0].next_poll;
    assert_eq!(caption::step(&l, Some(&api), due, Conditions::all()), Did::Took(0));
    let g = l.lock().unwrap();
    // A request it could not read: given up. Its trouble: tried again later.
    // Declined: not asked again, but its tokens were spent.
    assert!(matches!(&g.records["IMG_20261121_100000.jpg"].caption, Job::Failed(why) if why.contains("image too large")));
    assert!(matches!(g.records["IMG_20261121_100100.jpg"].caption, Job::Waiting { tries: 1, next } if next == due + 120));
    assert_eq!(g.records["IMG_20261121_100200.jpg"].caption, Job::Skipped("claude declined to caption it".into()));
    assert!(g.records["IMG_20261121_100200.jpg"].cost > 0);
    drop(g);
    // Eight tries, and it is given up.
    let mut job = Job::Waiting { tries: 0, next: 0 };
    for _ in 0..caption::TRIES {
        caption::retry(&mut job, 0, "busy");
    }
    assert!(matches!(job, Job::Failed(_)));
    assert_eq!(caption::backoff(20), 6 * 3600, "never more than six hours");
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(not(target_arch = "aarch64"))]
#[test]
fn the_monthly_limit_is_never_passed_and_then_captions_wait() {
    let dir = scratch("cap");
    let l = ledger(&dir);
    let now = 1_795_000_000;
    let names = ["IMG_20261121_100000.jpg", "IMG_20261121_100100.jpg", "IMG_20261121_100200.jpg"];
    waiting(&dir, &l, &names, now - 600);
    let (base, asked) = stand_in(Box::new(|a: &Asked| match a.method.as_str() {
        "POST" => (200, recorded::batch("msgbatch_3", "in_progress")),
        _ => (200, "{}".into()),
    }));
    let api = Api { base, key: KEY.into(), demo: false, stamp: 0 };
    let most = caption::most(1024, 769, true);
    let month = super::store::month_of(now);
    {
        let mut g = l.lock().unwrap();
        g.prefs.cap_cents = 100;
        // Room for one photo at the most it could cost, not two.
        let spent = g.cap() - most - most / 2;
        g.spend.insert(month.clone(), spent);
    }
    assert_eq!(caption::step(&l, Some(&api), now, Conditions::all()), Did::Sent(1));
    let body: Value = serde_json::from_str(&asked.lock().unwrap()[0].body).unwrap();
    assert_eq!(body["requests"].as_array().unwrap().len(), 1);
    assert_eq!(l.lock().unwrap().batches[0].reserved, most);
    // The rest wait for next month.
    assert_eq!(caption::step(&l, Some(&api), now + 1, Conditions::all()), Did::Waits("the month's limit is reached"));
    assert_eq!(asked.lock().unwrap().len(), 1);
    assert!(l.lock().unwrap().room(&month) < most);
    // The most a photo can cost, and what one usually does.
    assert_eq!(caption::image_tokens(1024, 769), 37 * 28);
    assert!(most < 20_000, "under 2¢ in a batch at the very most: {most}");
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(not(target_arch = "aarch64"))]
#[test]
fn a_caption_asked_for_at_once_uses_the_messages_api_with_its_fallback() {
    let dir = scratch("now");
    let l = ledger(&dir);
    let now = 1_795_000_000;
    waiting(&dir, &l, &["IMG_20261121_100000.jpg"], now - 30);
    let (base, asked) = stand_in(Box::new(|a: &Asked| match a.path.as_str() {
        "/v1/messages" => (200, recorded::message("Caption: The gearbox, opened.", 1290, 80).to_string()),
        _ => (404, String::new()),
    }));
    let api = Api { base, key: KEY.into(), demo: false, stamp: 0 };
    assert_eq!(caption::now_for(&l, &api, "IMG_20261121_100000.jpg", now).as_deref(), Ok("The gearbox, opened."));
    let a = asked.lock().unwrap()[0].clone();
    assert_eq!(a.header("anthropic-beta"), Some("server-side-fallback-2026-07-01"));
    let body: Value = serde_json::from_str(&a.body).unwrap();
    assert_eq!(body["fallbacks"], "default");
    assert_eq!(l.lock().unwrap().records["IMG_20261121_100000.jpg"].cost, 1290 * 4 + 80 * 20, "full price");
    // A pixel picture is never sent.
    assert!(caption::now_for(&l, &api, "PIX_20261121_100000.png", now).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_key_never_reaches_a_command_line_or_stays_in_a_file() {
    crate::shell::tests::sandbox();
    let (base, asked) = stand_in(Box::new(|_: &Asked| (200, "{}".into())));
    let reply = super::net::call("POST", &format!("{base}/v1/x"), &format!("x-api-key: {KEY}\n"), Some(b"{\"a\":1}"), "CatalystLinux-Photos/1.4").unwrap();
    assert_eq!(reply.code, 200);
    assert_eq!(asked.lock().unwrap()[0].header("x-api-key"), Some(KEY), "reached as a header, from curl's input");
    assert_eq!(asked.lock().unwrap()[0].body, "{\"a\":1}");
    // The body's file is gone, and no file there ever holds the key.
    let out = super::dir().join("out");
    for f in std::fs::read_dir(&out).map(|d| d.flatten().collect::<Vec<_>>()).unwrap_or_default() {
        assert!(!std::fs::read_to_string(f.path()).unwrap_or_default().contains(KEY));
    }
    for source in ["net.rs", "caption.rs"] {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/photolog").join(source)).unwrap();
        assert!(!text.contains("eprintln!") && !text.contains("println!"), "{source} logs nothing");
    }
    assert!(super::net::call("GET", "http://example.com/", "", None, "x").is_err(), "plain http only to this computer");
}

// --------------------------------------------------------- town and place ---

#[test]
fn the_gazetteer_names_the_town_offline() {
    assert!(super::gazetteer::towns() > 60_000);
    // Rochester Institute of Technology, where the Finger Lakes Regional is
    // held: the nearest town of 5,000, five km away, by its name alone.
    let rit = super::gazetteer::nearest(43.0848, -77.6744).unwrap();
    assert_eq!(rit.words, "West Henrietta, NY", "{rit:?}");
    assert!((rit.km - 5.1).abs() < 0.5);
    // Downtown Toronto, and somewhere far from any town.
    assert!(super::gazetteer::nearest(43.6532, -79.3832).unwrap().words.starts_with("Toronto, Ontario"));
    assert_eq!(super::gazetteer::nearest(-60.0, -140.0), None, "the Southern Ocean");
    assert_eq!(super::gazetteer::nearest(f64::NAN, 0.0), None);
}

#[test]
fn a_fingerprint_is_looked_up_at_beacondb_and_becomes_a_town() {
    let dir = scratch("town");
    let l = ledger(&dir);
    l.lock().unwrap().prefs.town = true;
    let fp = vec![ap(0x7c_10_c9_4a_31_20, -48), ap(0x3c_37_86_12_aa_01, -63), Ap { bssid: 0x11_22_33_44_55_66, dbm: -50, share: false }];
    for n in ["IMG_20261117_100000.jpg", "IMG_20261117_100400.jpg"] {
        std::fs::write(dir.join("photos").join(n), b"jpeg").unwrap();
        l.lock().unwrap().note_taken(n, 1_000, fp.clone(), None);
    }
    // One alone: too few to send.
    std::fs::write(dir.join("photos/IMG_20261117_110000.jpg"), b"jpeg").unwrap();
    l.lock().unwrap().note_taken("IMG_20261117_110000.jpg", 1_000, vec![ap(0x99, -40)], None);
    assert_eq!(l.lock().unwrap().records["IMG_20261117_110000.jpg"].town, Job::None);
    let (base, asked) = stand_in(Box::new(|_: &Asked| (200, json!({"location": {"lat": 43.0848, "lng": -77.6744}, "accuracy": 35.0}).to_string())));
    assert_eq!(locate::step(&l, &base, false, 2_000, Conditions::all()), Did::Took(2));
    let a = asked.lock().unwrap();
    assert_eq!(a.len(), 1, "the second photo, the same place, took the first one's town");
    let body: Value = serde_json::from_str(&a[0].body).unwrap();
    assert_eq!(body["considerIp"], false);
    assert_eq!(body["fallbacks"], json!({"lacf": false, "ipf": false}));
    let macs: Vec<&str> = body["wifiAccessPoints"].as_array().unwrap().iter().map(|w| w["macAddress"].as_str().unwrap()).collect();
    assert_eq!(macs, ["7c:10:c9:4a:31:20", "3c:37:86:12:aa:01"], "what may not be shared is not sent");
    assert_eq!(a[0].header("user-agent"), Some("CatalystLinux-Photos/1.4"));
    assert_eq!(a[0].path, "/v1/geolocate");
    assert!(a[0].header("x-api-key").is_none() && a[0].header("cookie").is_none());
    drop(a);
    assert_eq!(meta::read(&dir.join("photos/IMG_20261117_100400.jpg")).town, "West Henrietta, NY");
    // Unknown there (404) is no town; a guess from the IP is no position; off sends nothing.
    assert_eq!(locate::parse(404, &json!({"error": {"code": 404, "message": "Not found"}}).to_string()), Ok(None));
    assert_eq!(locate::parse(200, &json!({"location": {"lat": 1.0, "lng": 2.0}, "accuracy": 25000.0, "fallback": "ipf"}).to_string()), Ok(None));
    l.lock().unwrap().prefs.town = false;
    assert_eq!(locate::step(&l, &base, false, 3_000, Conditions::all()), Did::Waits("the town lookup is off"));
    // At a named place or an event, a photo needs no town: not looked up.
    l.lock().unwrap().prefs.town = true;
    l.lock().unwrap().places.name("Shop", &fp);
    l.lock().unwrap().note_taken("IMG_20261118_100000.jpg", 1_000, fp.clone(), None);
    assert_eq!(l.lock().unwrap().records["IMG_20261118_100000.jpg"].town, Job::None);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn beacondb_trouble_backs_off() {
    let dir = scratch("town-retry");
    let l = ledger(&dir);
    l.lock().unwrap().prefs.town = true;
    std::fs::write(dir.join("photos/IMG_1.jpg"), b"jpeg").unwrap();
    l.lock().unwrap().note_taken("IMG_1.jpg", 1_000, vec![ap(0x7c_10_c9_4a_31_20, -48), ap(0x3c_37_86_12_aa_01, -63)], None);
    let (base, _) = stand_in(Box::new(|_: &Asked| (503, String::new())));
    assert!(matches!(locate::step(&l, &base, false, 2_000, Conditions::all()), Did::Failed(_)));
    assert!(matches!(l.lock().unwrap().records["IMG_1.jpg"].town, Job::Waiting { tries: 1, next: 2_120 }));
    assert_eq!(locate::step(&l, &base, false, 2_000, Conditions { online: false, ..Conditions::all() }), Did::Waits("waiting to be online"));
    std::fs::remove_dir_all(&dir).ok();
}

// ----------------------------------------------------------- the sidecar ---

#[cfg(not(target_arch = "aarch64"))]
#[test]
fn the_sidecar_goes_there_and_back_and_the_photo_is_never_touched() {
    let dir = scratch("xmp");
    let path = photo(&dir, "IMG_20261121_100000.jpg", 320, 240);
    let before = std::fs::read(&path).unwrap();
    let about = About {
        caption: "Bumper bolts < 1/4\" & \"spare\" ones; it's the left side.\nSecond line, ünïcödé, 日本".into(),
        by: "claude-opus-5-5".into(),
        place: "Pit & queue".into(),
        event: "Finger Lakes Regional".into(),
        venue: "Rochester, NY".into(),
        context: "during Q31".into(),
        town: "Henrietta, NY".into(),
    };
    meta::write(&path, &about).unwrap();
    assert_eq!(meta::read(&path), about);
    assert_eq!(std::fs::read(&path).unwrap(), before, "the JPEG is the camera's, byte for byte");
    assert!(crate::photos::decode(&path, 80, 60).is_ok());
    let xmp = std::fs::read_to_string(meta::sidecar(&path)).unwrap();
    assert!(meta::sidecar(&path).ends_with("IMG_20261121_100000.xmp"));
    // The fields other programs read.
    for want in ["<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">Bumper bolts &lt; 1/4&quot;", "<Iptc4xmpCore:Location>Pit &amp; queue</Iptc4xmpCore:Location>", "<photoshop:City>Henrietta</photoshop:City>", "<?xpacket end=\"w\"?>"] {
        assert!(xmp.contains(want), "{want} in\n{xmp}");
    }
    assert!(!xmp.contains("7c:10"), "no wi-fi in a sidecar");
    // Another program's sidecar, its fields as attributes.
    let theirs = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF><rdf:Description rdf:about=\"\" photoshop:City=\"Rochester\" Iptc4xmpCore:Location=\"RIT\"><dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">From Lightroom</rdf:li></rdf:Alt></dc:description></rdf:Description></rdf:RDF></x:xmpmeta>";
    let read = About::from_xmp(theirs);
    assert_eq!((read.caption.as_str(), read.place.as_str(), read.town.as_str()), ("From Lightroom", "RIT", "Rochester"));
    // Nothing to say: no sidecar.
    meta::write(&path, &About::default()).unwrap();
    assert!(!meta::sidecar(&path).exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn edits_take_their_photos_caption_and_a_renamed_moment_finds_its_photo() {
    let dir = scratch("reconcile");
    let l = ledger(&dir);
    let photos = dir.join("photos");
    std::fs::write(photos.join("IMG_20261121_100000.jpg"), b"jpeg").unwrap();
    meta::write(&photos.join("IMG_20261121_100000.jpg"), &About { caption: "The intake.".into(), by: "claude-opus-5-5".into(), place: "Shop".into(), ..About::default() }).unwrap();
    std::fs::write(photos.join("IMG_20261121_100000_edit.jpg"), b"jpeg").unwrap();
    // A moment noted as IMG_…_101500.jpg, written by the camera as _2.
    l.lock().unwrap().note_taken("IMG_20261121_101500.jpg", 5_000, places::demo_place("shop"), None);
    std::fs::write(photos.join("IMG_20261121_101500_2.jpg"), b"jpeg").unwrap();
    let names = vec![("IMG_20261121_100000.jpg".to_owned(), 4_000), ("IMG_20261121_100000_edit.jpg".to_owned(), 4_010), ("IMG_20261121_101500_2.jpg".to_owned(), 5_001)];
    assert!(l.lock().unwrap().reconcile(&names));
    let g = l.lock().unwrap();
    assert_eq!(meta::read(&photos.join("IMG_20261121_100000_edit.jpg")).caption, "The intake.");
    assert_eq!(g.records["IMG_20261121_100000_edit.jpg"].caption, Job::Done);
    assert_eq!(g.records["IMG_20261121_101500_2.jpg"].fp, places::demo_place("shop"), "the moment's Wi-Fi went to the photo that was written");
    assert!(!g.records.contains_key("IMG_20261121_101500.jpg"));
    assert_eq!(super::store::edit_of("IMG_1_edit_2.jpg").as_deref(), Some("IMG_1.jpg"));
    assert_eq!(super::store::edit_of("IMG_1_editor.jpg"), None);
    assert!(super::captionable("PIX_20261121_100000.png").is_err() && super::captionable("IMG_1_edit.jpg").is_err() && super::captionable("IMG_1.jpg").is_ok());
    drop(g);
    std::fs::remove_dir_all(&dir).ok();
}

// --------------------------------------------------------------- the log ---

fn entry(stamp: &str, about: About) -> log::Entry {
    let at = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d_%H%M%S").unwrap();
    let name = format!("IMG_{stamp}.jpg");
    log::Entry { path: PathBuf::from(&name), name, at, about }
}

fn album() -> Vec<log::Entry> {
    let event = |caption: &str, place: &str, context: &str| About { caption: caption.into(), place: place.into(), event: "Finger Lakes Regional".into(), venue: "Rochester, NY".into(), context: context.into(), ..About::default() };
    vec![
        entry("20261121_154210", event("Bumpers back on.", "Pit", "before Q32")),
        entry("20261121_143005", event("On the field.", "", "during Q31")),
        entry("20261121_102214", event("Swapping bumper bolts.", "Pit", "")),
        entry("20261119_184500", About { caption: "New swerve modules fitted.".into(), by: "claude-opus-5-5".into(), place: "Shop".into(), ..About::default() }),
        entry("20261119_172201", About { caption: "Elevator *top* stop [test].".into(), place: "Shop".into(), ..About::default() }),
        entry("20261117_101532", About { town: "Henrietta, NY".into(), ..About::default() }),
        entry("20261117_090000", About::default()),
    ]
}

#[test]
fn the_log_is_days_newest_first_then_places_in_the_order_the_day_reached_them() {
    let days = log::group(album());
    let dates: Vec<String> = days.iter().map(|d| d.date.to_string()).collect();
    assert_eq!(dates, ["2026-11-21", "2026-11-19", "2026-11-17"]);
    let titles: Vec<&str> = days[0].groups.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(titles, ["Pit", "Finger Lakes Regional · Rochester, NY"], "the pit first: the day started there");
    let pit: Vec<&str> = days[0].groups[0].entries.iter().map(|e| e.about.caption.as_str()).collect();
    assert_eq!(pit, ["Swapping bumper bolts.", "Bumpers back on."], "in the order they were taken");
    assert_eq!(days[0].count(), 3);
    assert_eq!(days[0].event(), Some("Finger Lakes Regional"));
    let last: Vec<&str> = days[2].groups.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(last, ["", "Henrietta, NY"]);
    // A range: the day, its event, its week, everything.
    let at = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
    assert_eq!(log::select(&days, Range::Day, at).len(), 1);
    assert_eq!(log::select(&days, Range::Event, at).len(), 1);
    assert_eq!(log::select(&days, Range::Week, at).len(), 3);
    assert_eq!(log::select(&days, Range::Week, chrono::NaiveDate::from_ymd_opt(2026, 11, 18).unwrap()).len(), 1);
    assert_eq!(log::select(&days, Range::All, at).len(), 3);
}

#[test]
fn a_log_exports_as_markdown_with_its_photos() {
    let dir = scratch("export");
    let photos = dir.join("photos");
    let mut entries = album();
    for e in &mut entries {
        e.path = photos.join(&e.name);
        std::fs::write(&e.path, format!("jpeg of {}", e.name)).unwrap();
    }
    let days = log::group(entries);
    let at = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
    let chosen = log::select(&days, Range::Week, at);
    let root = photos.join("Build log");
    let (folder, n) = log::export(&chosen, Range::Week, &root, true).unwrap();
    assert_eq!(n, 7);
    assert_eq!(folder.file_name().unwrap(), "2026-11-17 to 2026-11-21");
    let md = std::fs::read_to_string(folder.join("build-log.md")).unwrap();
    // Oldest first, as a notebook reads; each photo, its time, its caption.
    let first = md.find("## Tuesday 17 November 2026").unwrap();
    let last = md.find("## Saturday 21 November 2026").unwrap();
    assert!(first < last, "{md}");
    assert!(md.starts_with("# Build log · 17 Nov to 21 Nov 2026\n"));
    assert!(md.contains("### Shop\n"));
    assert!(md.contains("![Swapping bumper bolts.](images/IMG_20261121_102214.jpg)"));
    assert!(md.contains("**10:22** · Swapping bumper bolts.\n\n*Finger Lakes Regional*"), "the event under a named place's photo\n{md}");
    assert!(md.contains("**14:30** · On the field.\n\n*during Q31*"));
    assert!(md.contains("Elevator \\*top\\* stop \\[test\\]."), "markdown in a caption reads as itself");
    assert!(md.contains("**18:45** · New swerve modules fitted. *· Claude*"), "claude's captions say so");
    assert!(md.contains("### Elsewhere"));
    // The photos beside it, the same bytes.
    for name in ["IMG_20261121_154210.jpg", "IMG_20261117_090000.jpg"] {
        assert_eq!(std::fs::read(folder.join("images").join(name)).unwrap(), std::fs::read(photos.join(name)).unwrap());
    }
    // An event's export is named for it; again over the same folder is fine.
    let (event_folder, n) = log::export(&log::select(&days, Range::Event, at), Range::Event, &root, false).unwrap();
    assert_eq!(n, 3);
    assert_eq!(event_folder.file_name().unwrap(), "2026-11-21 Finger Lakes Regional");
    assert!(std::fs::read_to_string(event_folder.join("build-log.md")).unwrap().contains("**2:30 pm**"));
    assert!(log::export(&log::select(&days, Range::Event, at), Range::Event, &root, false).is_ok());
    assert!(log::export(&[], Range::Day, &root, true).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_day_goes_to_team_as_a_few_short_messages() {
    let mut entries = album();
    entries[1].about.caption = "x".repeat(400);
    let days = log::group(entries);
    let texts = log::team_texts(&days[0], true);
    assert_eq!(texts.len(), 4);
    assert!(texts[0].starts_with("build log · sat 21 nov") && texts[0].contains("3 photos") && texts[0].ends_with("pit, finger lakes regional"), "{}", texts[0]);
    assert!(texts.iter().all(|t| t.chars().count() <= crate::team::wire::TEXT_MOST));
    assert_eq!(texts[1], "10:22 Swapping bumper bolts.");
    assert!(texts[2].starts_with("14:30 (during Q31) xxx") && texts[2].ends_with('…'));
    // A day with no captions is its one line.
    assert_eq!(log::team_texts(&days[2], true).len(), 1);
    // And not in a team: said so, nothing sent.
    if !crate::team::member() {
        assert_eq!(super::ui::send_to_team(&days[0], true), "join a team first, in Team");
    }
}

// ------------------------------------------------------------ the screens ---

#[test]
fn every_page_s_keys_fit_the_sheet_and_its_rows_fit_the_panel() {
    crate::shell::tests::sandbox();
    let mut g = crate::gfx::Gfx::new(bezel::Tokens::dark());
    let mut fits = |groups: Vec<crate::help::KeyGroup>| crate::keysheet::fits(&mut g, &groups).unwrap();
    use crate::shell::Screen;
    fits(super::ui::PhotoSettings::new().key_sheet());
    fits(super::ui::PlacesPage::new().key_sheet());
    fits(super::ui::NamePlace::new("IMG_1.jpg").key_sheet());
    fits(super::ui::CaptionPage::new(Path::new("IMG_1.jpg"), "").key_sheet());
    fits(super::ui::RenamePlace::new(1, "Shop").key_sheet());
    fits(super::ui::ExportPage::new(Vec::new(), chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap()).key_sheet());
    let mut under = super::Under::new();
    fits(under.key_groups());
    let mut robot = crate::robot::Robot::none();
    let mut settings = crate::shell::Settings::default();
    let mut say = None;
    under.key(crate::platform::Key::Down, &mut crate::shell::Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
    fits(under.key_groups());
    // The notes under Settings › Photos's switches are whole at every size.
    let mut g = crate::gfx::Gfx::new(bezel::Tokens::dark());
    for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
        g.set_text_size(size);
        let style = g.ty.caption;
        let room = crate::gfx::W - 2.0 * crate::gfx::PAD - 12.0;
        for choices in [["photos are sent to anthropic to be captioned", "photos are sent to anthropic"], ["sends nearby wi-fi addresses to beacondb.net", "sends nearby wi-fi to beacondb.net"]] {
            assert!(choices.iter().any(|c| g.measure(c, style) <= room), "{choices:?} at {size:?}");
        }
    }
}

#[test]
fn records_are_kept_privately_and_read_back_whole() {
    let dir = scratch("ledger");
    let l = ledger(&dir);
    {
        let mut g = l.lock().unwrap();
        g.prefs.captions = true;
        g.prefs.cap_cents = 500;
        g.records.insert("IMG_1.jpg".into(), Record { at: 5, fp: places::demo_place("home"), caption: Job::Sent("msgbatch_x".into()), town: Job::Failed("no".into()), cost: 4_000, spot: Some(super::store::Spot { lat: 1.5, lon: -2.5, accuracy: 30.0 }) });
        g.batches.push(super::store::Batch { id: "msgbatch_x".into(), sent: 1, next_poll: 2, polls: 3, items: vec![("IMG_1-0".into(), "IMG_1.jpg".into())], reserved: 9 });
        g.spend.insert("2026-11".into(), 12_345);
        g.save();
    }
    let again = Ledger::open(&dir.join("private"), &dir.join("photos"));
    let g = l.lock().unwrap();
    assert_eq!((again.prefs.clone(), again.records.clone(), again.batches.clone(), again.spend.clone()), (g.prefs.clone(), g.records.clone(), g.batches.clone(), g.spend.clone()));
    assert_eq!(caption::money(4_000), "0.4¢");
    assert_eq!(caption::money(3_000_000), "$3.00");
    drop(g);
    std::fs::remove_dir_all(&dir).ok();
}

/// One real caption, of the simulator's field, from Anthropic itself: only
/// when asked for (`--ignored`) and only with a key in `ANTHROPIC_API_KEY`.
#[cfg(not(target_arch = "aarch64"))]
#[test]
#[ignore]
fn a_real_caption() {
    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
        eprintln!("no ANTHROPIC_API_KEY: skipped");
        return;
    };
    let dir = scratch("real");
    let l = ledger(&dir);
    waiting(&dir, &l, &["IMG_20261121_100000.jpg"], 1_795_000_000);
    let api = Api { base: caption::ANTHROPIC.into(), key, demo: false, stamp: 0 };
    let text = caption::now_for(&l, &api, "IMG_20261121_100000.jpg", chrono::Utc::now().timestamp()).unwrap();
    let cost = l.lock().unwrap().records["IMG_20261121_100000.jpg"].cost;
    println!("caption: {text}\ncost: {}", caption::money(cost));
    assert!(!text.is_empty() && text.chars().count() <= caption::CAPTION_MOST);
    std::fs::remove_dir_all(&dir).ok();
}
