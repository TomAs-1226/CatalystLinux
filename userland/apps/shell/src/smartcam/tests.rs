use super::frame::*;
use super::*;

#[test]
fn crc32_is_the_standard_one() {
    // The check value every CRC-32 (IEEE) has for "123456789".
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

#[test]
fn the_wire_bytes_are_the_cameras() {
    // tools/unitv/test_protocol.py checks main.py makes these same bytes.
    let bytes = encode(RESULTS, 1, br#"{"n":1}"#);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, "c57a85010700e77b226e223a317d5ed2d160");
}

#[test]
fn frames_survive_junk_a_split_read_and_a_corrupted_neighbour() {
    let a = encode(RESULTS, 1, br#"{"n":1}"#);
    let b = encode(PREVIEW, 2, &[0xC5, 0x7A, 1, 2, 3, 0xC5]);
    let mut bad = encode(PONG, 3, b"x");
    let last = bad.len() - 1;
    bad[last] ^= 0x40;
    let c = encode(LOG, 4, b"after the bad one");
    let mut wire = b"boot noise \xc5".to_vec();
    wire.extend_from_slice(&a);
    wire.extend_from_slice(&[0x00, 0xC5, 0xC5, 0x7A]);
    wire.extend_from_slice(&b);
    wire.extend_from_slice(&bad);
    wire.extend_from_slice(&c);
    // Every way of cutting it up gives the same frames.
    for cut in [1usize, 2, 3, 5, 7, 11, 64, wire.len()] {
        let mut d = Decoder::default();
        let mut got = Vec::new();
        for chunk in wire.chunks(cut) {
            got.extend(d.push(chunk));
        }
        let kinds: Vec<(u8, u8)> = got.iter().map(|f| (f.kind, f.seq)).collect();
        assert_eq!(kinds, vec![(RESULTS, 1), (PREVIEW, 2), (LOG, 4)], "cut into {cut}s");
        assert_eq!(got[1].payload, vec![0xC5, 0x7A, 1, 2, 3, 0xC5], "a payload that holds the sync word");
        assert!(d.bad >= 1, "the corrupted frame was refused");
        assert!(d.junk > 0);
    }
}

#[test]
fn a_false_sync_with_a_huge_length_is_refused_at_once() {
    let good = encode(PONG, 9, b"ok");
    // A sync word in junk, a header claiming 60 kB: refused by the
    // header's own check, not waited on.
    let mut wire = vec![0xC5, 0x7A, 0x85, 0x01, 0x60, 0xEA, 0x00];
    wire.extend_from_slice(&good);
    let mut d = Decoder::default();
    let got = d.push(&wire);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].payload, b"ok");
}

#[test]
fn power_is_on_while_wanted_warm_after_and_then_off() {
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let mut p = Power::Off;
    assert_eq!(p.step(at(0), false, false), Switch::Keep, "nobody wants it: off");
    assert_eq!(p.step(at(10), true, false), Switch::On, "wanted: on at once");
    assert_eq!(p.step(at(500), true, false), Switch::Keep);
    assert!(matches!(p, Power::Waking { .. }));
    // It answers 1.4 s after power: that is its start time.
    p.step(at(1410), true, true);
    assert_eq!(p, Power::On { took: Duration::from_millis(1400) });
    // Let go: warm for WARM_FOR, wanted again meanwhile, no restart.
    assert_eq!(p.step(at(5000), false, false), Switch::Keep);
    assert!(matches!(p, Power::Warm { .. }));
    assert_eq!(p.step(at(9000), true, false), Switch::Keep, "back within the warm time: still on");
    assert!(matches!(p, Power::On { .. }));
    p.step(at(10_000), false, false);
    let off_at = 10_000 + WARM_FOR.as_millis() as u64;
    assert_eq!(p.step(at(off_at - 1), false, false), Switch::Keep);
    assert_eq!(p.step(at(off_at), false, false), Switch::Off, "off once the warm time is out");
    assert_eq!(p, Power::Off);
}

#[test]
fn a_camera_that_never_answers_is_powered_off_and_tried_again_later() {
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let mut p = Power::Off;
    p.step(at(0), true, false);
    let timeout = WAKE_TIMEOUT.as_millis() as u64;
    assert_eq!(p.step(at(timeout), true, false), Switch::Off);
    assert!(matches!(p, Power::Failed { .. }));
    assert_eq!(p.step(at(timeout + 1000), true, false), Switch::Keep, "not hammered");
    assert_eq!(p.step(at(timeout + RETRY_AFTER.as_millis() as u64), true, false), Switch::On, "tried again");
    // Nobody wants it any more: off, and stays so.
    let mut q = Power::Failed { at: at(0) };
    q.step(at(10), false, false);
    assert_eq!(q, Power::Off);
}

#[test]
fn results_are_read_into_the_cameras_picture() {
    let r = parse_results(br#"{"n":42,"w":320,"h":240,"fps":14.5,"f":[[160,120,32,24,91]],"q":[[0,0,64,48,"SCOUT 5805"]],"a":[[7,80,60,250,-412,-88,2640]],"b":[[0,40,176,34,28,640]]}"#).unwrap();
    assert_eq!(r.frame, 42);
    assert_eq!(r.faces, vec![Face { at: [0.5, 0.5, 0.6, 0.6], score: 0.91 }]);
    assert_eq!(r.codes[0].text, "SCOUT 5805");
    assert_eq!(r.tags[0].id, 7);
    assert!((r.tags[0].turn - 0.25).abs() < 1e-6 && r.tags[0].pose_mm == [-412.0, -88.0, 2640.0]);
    assert_eq!(r.blobs[0].pixels, 640);
    assert!(parse_results(b"not json").is_none());
}

#[test]
fn a_face_maps_into_the_main_cameras_picture() {
    let a = Align::default();
    // The wider lens: its picture's middle is the main one's, its edges
    // further out.
    assert!(a.sx > 1.05 && a.sx < 1.2 && a.sy > 1.05 && a.sy < 1.2, "{a:?}");
    let mid = a.map([0.45, 0.45, 0.55, 0.55]);
    assert!((mid[0] - (0.5 - 0.05 * a.sx)).abs() < 1e-5);
    let offset = Align { ox: 0.1, ..a };
    assert!((offset.map([0.5, 0.5, 0.5, 0.5])[0] - 0.6).abs() < 1e-5);
    // The largest sure face, if it falls in the main picture.
    let r = Results { faces: vec![Face { at: [0.4, 0.4, 0.5, 0.5], score: 0.9 }, Face { at: [0.1, 0.1, 0.12, 0.12], score: 0.95 }, Face { at: [0.3, 0.3, 0.7, 0.7], score: 0.3 }], ..Results::default() };
    let f = main_face(&r, &a).unwrap();
    assert!(f[0] > 0.35 && f[2] < 0.6);
    let outside = Results { faces: vec![Face { at: [0.0, 0.0, 0.03, 0.03], score: 0.9 }], ..Results::default() };
    assert!(main_face(&outside, &a).is_none(), "a face beyond the main picture's edge");
}

/// The whole link against the stand-in: woken, its start time measured,
/// the fastest rate taken, results and previews arriving, then warm.
#[test]
fn the_stand_in_wakes_and_answers_at_the_fastest_rate() {
    let _port = sim::exclusive(true);
    // The count goes on from the tests before (the Camera's use the same).
    let before = preview().0;
    let lease = lease(Use::View);
    let started = Instant::now();
    while !matches!(status(), Status::Ready { baud: 3_000_000, .. }) && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let Status::Ready { baud, woke_ms } = status() else { panic!("{:?}", status()) };
    assert_eq!(baud, 3_000_000);
    assert!((1300..4000).contains(&woke_ms), "it took {woke_ms} ms to start");
    // Room for a busy machine: the whole suite runs beside it.
    while (preview().0 < before + 3 || results(Duration::from_secs(1)).is_none()) && started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(preview().0 >= before + 3, "previews arrive");
    let r = results(Duration::from_secs(5)).expect("results arrive");
    assert!(!r.faces.is_empty() && !r.codes.is_empty() && !r.tags.is_empty());
    assert!(tags().is_some_and(|(_, t)| t[0].id == 7), "the field view's tags");
    drop(lease);
    std::thread::sleep(Duration::from_millis(200));
    assert!(matches!(status(), Status::Ready { .. }), "warm: still answering a moment after");
}

/// The port changes hands at once, both ways: to the console, the
/// camera in use is let go of and unpowered at once (no warm time), and
/// nothing powers it while the port changes; back, it wakes again with
/// no restart.
#[test]
fn the_port_switches_at_once_and_lets_go_first() {
    let _port = sim::exclusive(true);
    let _lease = lease(Use::View);
    let wait = |what: &dyn Fn() -> bool| {
        let started = Instant::now();
        while !what() && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        what()
    };
    assert!(wait(&|| matches!(status(), Status::Ready { .. })), "{:?}", status());
    switch_grove(false);
    assert!(wait(&|| !switching()), "the switch finished");
    assert!(!chosen());
    assert!(!PORT_OPEN.load(Ordering::Acquire));
    assert!(wait(&|| status() == Status::PortIsConsole), "unpowered at once, no warm time: {:?}", status());
    switch_grove(true);
    assert!(wait(&|| !switching()));
    assert!(chosen());
    assert!(wait(&|| matches!(status(), Status::Ready { .. })), "awake again without a restart: {:?}", status());
}

// ------------------------------------------- the link's start, its rate, the update ---

/// Quick waits, for a stand-in that starts in 300 ms.
const QUICK: Timing = Timing { answer: Duration::from_millis(1500), off_for: Duration::from_millis(30), ack: Duration::from_millis(500), settle: Duration::from_millis(1200) };

fn stand_in(spec: sim::Spec) -> (Link, Arc<Mutex<sim::Flash>>) {
    let (port, flash) = sim::port_with(spec);
    let mut link = Link::new(Box::new(port));
    link.port.power(true);
    (link, flash)
}

fn quick(spec: sim::Spec) -> sim::Spec {
    sim::Spec { boot: Duration::from_millis(300), ..spec }
}

fn small_payload() -> crate::unitv::Payload {
    let made = |n: usize, seed: u8| (0..n).map(|i| (i as u8).wrapping_mul(29).wrapping_add(seed)).collect::<Vec<u8>>();
    crate::unitv::Payload { main: made(30_000, 1), boot: made(2_000, 2), model: made(60_000, 3), firmware: Vec::new(), firmware_name: String::new(), isp: Vec::new() }
}

const OLD_MAIN: &[u8] = b"# the program it came with\n";

/// An update against a stand-in as `spec` says: how it went, its /flash,
/// what the progress said, and the stamp sent.
fn update_with(spec: sim::Spec) -> (Result<update::Sent, String>, Arc<Mutex<sim::Flash>>, Vec<Update>, crate::unitv::Stamp) {
    let (mut link, flash) = stand_in(quick(spec));
    let hello = link.hello(Duration::from_secs(3)).expect("heard at 115 200");
    let h = Hello::parse(&hello.payload).expect("its hello");
    let p = small_payload();
    let stamp = p.stamp("MaixPy 0.6.3 (simulated)");
    let update::Plan::Send { program, model } = update::plan(&h, Some(&stamp)) else { panic!("{:?}", update::plan(&h, Some(&stamp))) };
    climb(&mut link, &QUICK);
    let mut seen = Vec::new();
    let r = update::run(&mut link, &p, &stamp, program, model, &QUICK, &mut |u| seen.push(u));
    (r, flash, seen, stamp)
}

#[test]
fn a_slow_start_is_waited_for() {
    // MaixPy, the sensor and the face model can take seconds: the first
    // connect waits at least eight.
    assert!(WAKE_TIMEOUT >= Duration::from_secs(8));
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let mut p = Power::Off;
    p.step(at(0), true, false);
    assert_eq!(p.step(at(7900), true, false), Switch::Keep, "not given up on at 7.9 s");
    p.step(at(9000), true, true);
    assert_eq!(p, Power::On { took: Duration::from_millis(9000) });
    // Over the wire: a stand-in 2.5 s from power to listening is heard, and
    // says what it is.
    let (mut link, _) = stand_in(sim::Spec { boot: Duration::from_millis(2500), ..sim::Spec::bare() });
    let began = Instant::now();
    let hello = link.hello(Duration::from_secs(8)).expect("heard");
    assert!(began.elapsed() >= Duration::from_millis(2400), "{:?}", began.elapsed());
    let h = Hello::parse(&hello.payload).expect("its hello");
    assert_eq!((h.version, h.updates, h.full, h.cwd.as_str()), (2, true, Some(true), "/flash"));
    assert!(link.got > 0 && link.sent > 0, "the bytes counted");
}

#[test]
fn the_rate_climbs_only_as_far_as_both_ends_hold() {
    for (holds, settles, failed) in [(3_000_000, 3_000_000, None), (1_000_000, 1_000_000, Some(1_500_000)), (START_BAUD, START_BAUD, Some(460_800))] {
        let (mut link, _) = stand_in(quick(sim::Spec { max_baud: holds, ..sim::Spec::bare() }));
        link.hello(Duration::from_secs(3)).expect("heard at 115 200");
        let c = climb(&mut link, &QUICK);
        assert_eq!((c.baud, c.failed), (settles, failed), "a camera that holds {holds}");
        assert_eq!(link.baud, settles);
        // Both ends where it settled: a ping at that rate is answered.
        assert!(link.ask(frame::PING, b"x", frame::PONG, Duration::from_millis(500), 2).is_some(), "both at {settles}");
    }
}

#[test]
fn hellos_old_and_new_are_read() {
    let old = Hello::parse(br#"{"v":1,"fw":"catalyst-unitv 1","kpu":"face yolo2 320x240","crc":"native","fov":80,"w":320,"h":240,"up_ms":2210,"usb":false,"stamp":{"catalyst":"0.9.3","main":"0a1b2c3d4e5f","model":"abcdefabcdef","firmware":"x"}}"#).expect("0.9.3's");
    assert_eq!((old.version, old.updates, old.full, old.up_ms), (1, false, None, 2210));
    assert_eq!(old.stamp.as_ref().map(|s| s.catalyst.as_str()), Some("0.9.3"));
    let new = Hello::parse(br#"{"v":2,"upd":1,"mp":"0.6.3","full":0,"sensor":"failed: Reset Failed","trial":1,"rolled_back":true,"cwd":"/sd","stamp":null}"#).expect("0.9.4's");
    assert_eq!((new.updates, new.full, new.trial, new.rolled_back), (true, Some(false), Some(1), true));
    assert_eq!(new.sensor.as_deref(), Some("failed: Reset Failed"));
    assert!(new.stamp.is_none());
}

#[test]
fn the_plan_takes_only_what_the_camera_can() {
    use crate::unitv::Stamp;
    let e = Stamp { catalyst: "0.9.4".into(), main: "111111111111".into(), model: "222222222222".into(), firmware: String::new() };
    let v2 = Hello { version: 2, updates: true, full: Some(true), cwd: "/flash".into(), stamp: Some(Stamp { catalyst: "0.9.3".into(), main: "000000000000".into(), ..e.clone() }), ..Hello::default() };
    assert_eq!(update::plan(&v2, Some(&e)), update::Plan::Send { program: true, model: false });
    let model_only = Hello { stamp: Some(Stamp { model: "333333333333".into(), ..e.clone() }), ..v2.clone() };
    assert_eq!(update::plan(&model_only, Some(&e)), update::Plan::Send { program: false, model: true });
    let unstamped = Hello { stamp: None, ..v2.clone() };
    assert_eq!(update::plan(&unstamped, Some(&e)), update::Plan::Send { program: true, model: true });
    assert_eq!(update::plan(&Hello { stamp: Some(e.clone()), ..v2.clone() }, Some(&e)), update::Plan::Nothing, "up to date");
    assert_eq!(update::plan(&v2, None), update::Plan::Nothing, "no program in the image");
    // What only USB-A can do: 0.9.3's program, a MaixPy short of what it
    // needs (even with the program up to date), a card it starts from.
    assert!(matches!(update::plan(&Hello { updates: false, version: 1, ..v2.clone() }, Some(&e)), update::Plan::UsbOnce(w) if w.contains("USB-A")));
    assert!(matches!(update::plan(&Hello { full: Some(false), stamp: Some(e.clone()), ..v2.clone() }, Some(&e)), update::Plan::UsbOnce(w) if w.contains("MaixPy")));
    assert!(matches!(update::plan(&Hello { cwd: "/sd".into(), ..v2.clone() }, Some(&e)), update::Plan::UsbOnce(w) if w.contains("microSD")));
}

#[test]
fn an_update_goes_over_in_chunks_and_the_camera_starts_again_with_it() {
    let (r, flash, seen, stamp) = update_with(sim::Spec::old());
    let sent = r.expect("updated");
    let p = small_payload();
    let f = flash.lock().unwrap();
    assert_eq!(f.files["main.py"], p.main);
    assert_eq!(f.files["boot.py"], p.boot);
    assert_eq!(f.files["face.kmodel"], p.model, "the model too: its stamp differed");
    assert_eq!(f.old["main.py"], OLD_MAIN, "the old program kept for a rollback");
    assert!(f.trial.is_none(), "the new program answered: its trial over");
    assert!(sent.hello.stamp.as_ref().is_some_and(|s| s.same_program(&stamp)), "{:?}", sent.hello.stamp);
    assert!(sent.resumed.is_empty() && sent.again == 0);
    // 92 kB in chunks of 4 kB at 3 Mbaud; the progress rose to the whole,
    // then the restart.
    assert!(f.chunks >= 23, "{} chunks", f.chunks);
    let dones: Vec<f32> = seen.iter().filter_map(|u| if let Update::Sending { done, .. } = u { Some(*done) } else { None }).collect();
    assert!(dones.windows(2).all(|w| w[1] >= w[0]) && dones.last().is_some_and(|d| (*d - 1.0).abs() < 1e-6), "{dones:?}");
    assert_eq!(seen.last(), Some(&Update::Restarting));
}

#[test]
fn a_chunk_lost_on_the_wire_or_written_wrong_is_sent_again() {
    let (r, flash, ..) = update_with(sim::Spec { drop_chunk: Some(3), ..sim::Spec::old() });
    r.expect("a lost chunk sent again");
    assert_eq!(flash.lock().unwrap().files["main.py"], small_payload().main);
    let (r, flash, ..) = update_with(sim::Spec { bad_write: Some(4), ..sim::Spec::old() });
    let sent = r.expect("a file that failed its CRC sent again");
    let f = flash.lock().unwrap();
    assert_eq!(f.damaged, 1, "its CRC refused it once");
    assert!(sent.again >= 1);
    assert_eq!(f.files["main.py"], small_payload().main);
    assert_eq!(f.files["face.kmodel"], small_payload().model);
}

#[test]
fn a_transfer_cut_off_resumes_where_it_stopped() {
    // The power goes once the model has 40 kB of its 60: the camera starts
    // again, is found again, and takes the rest from there.
    let (r, flash, ..) = update_with(sim::Spec { cut_at: Some(40_000), ..sim::Spec::old() });
    let sent = r.expect("resumed and finished");
    let (name, from) = sent.resumed.first().cloned().expect("resumed");
    assert_eq!(name, "face.kmodel");
    assert!((40_000..60_000).contains(&from), "from {from}");
    let f = flash.lock().unwrap();
    assert_eq!(f.files["face.kmodel"], small_payload().model);
    assert!(f.starts >= 3, "power, the cut, the new program: {} starts", f.starts);
}

#[test]
fn a_new_program_that_never_starts_is_rolled_back() {
    let (r, flash, ..) = update_with(sim::Spec { breaks_new: true, ..sim::Spec::old() });
    let sent = r.expect("answered again, with the old program");
    assert!(sent.hello.rolled_back, "it says so");
    assert_eq!(sent.hello.stamp.as_ref().map(|s| s.main.as_str()), Some("0a1b2c3d4e5f"), "the old stamp back");
    let f = flash.lock().unwrap();
    assert_eq!(f.files["main.py"], OLD_MAIN);
    assert!(!f.files.contains_key("face.kmodel"), "a model that was not a file before is gone again");
    assert!(f.starts >= 4, "two starts that never answered, then the old one: {} starts", f.starts);
}
