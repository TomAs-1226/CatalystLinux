//! Gamepad against event streams built here, as the kernel would send them,
//! and the health measures against numbers worked by hand.

use super::evdev::*;
use super::health::{self, Rest, Verdict};
use super::pads::{Pads, Sim};
use super::*;
use crate::shell::Screen;

/// An Xbox pad as xpad describes it.
fn xbox() -> Pad {
    Sim::pad()
}

/// A stream: each report's events, then its SYN_REPORT, `every` µs apart.
fn stream(reports: &[&[(u16, u16, i32)]], every: i64) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (i, report) in reports.iter().enumerate() {
        let at = 1_000_000 + i as i64 * every;
        for (kind, code, value) in report.iter() {
            bytes.extend_from_slice(&Event::new(at, *kind, *code, *value).bytes());
        }
        bytes.extend_from_slice(&Event::new(at, EV_SYN, SYN_REPORT, 0).bytes());
    }
    bytes
}

#[test]
fn events_read_from_the_kernels_layout() {
    // One record by hand: 5 s 250000 µs, EV_ABS, ABS_X, −32768.
    let mut raw = Vec::new();
    raw.extend_from_slice(&5i64.to_le_bytes());
    raw.extend_from_slice(&250_000i64.to_le_bytes());
    raw.extend_from_slice(&3u16.to_le_bytes());
    raw.extend_from_slice(&0u16.to_le_bytes());
    raw.extend_from_slice(&(-32768i32).to_le_bytes());
    // And half of the next: not taken until the rest arrives.
    raw.extend_from_slice(&[1, 2, 3, 4, 5]);
    let (events, used) = parse(&raw);
    assert_eq!(used, EVENT_SIZE);
    assert_eq!(events, vec![Event { sec: 5, usec: 250_000, kind: EV_ABS, code: ABS_X, value: -32768 }]);
    assert_eq!(events[0].micros(), 5_250_000);
    // And back, byte for byte.
    assert_eq!(events[0].bytes().as_slice(), &raw[..EVENT_SIZE]);
}

#[test]
fn a_pad_follows_its_reports() {
    let mut pad = xbox();
    let bytes = stream(&[&[(EV_ABS, ABS_X, 16384), (EV_ABS, ABS_Y, -32768), (EV_KEY, BTN_SOUTH, 1)], &[(EV_ABS, ABS_Z, 1023), (EV_ABS, ABS_HAT0X, -1)], &[(EV_KEY, BTN_SOUTH, 0), (EV_KEY, BTN_TR, 1)]], 4000);
    let (events, _) = parse(&bytes);
    assert!(pad.feed(&events[..4]));
    let (x, y) = pad.stick(0);
    assert!((x - 0.5).abs() < 0.001 && (y + 1.0).abs() < 1e-6, "left stick at ({x}, {y})");
    assert!(pad.down(BTN_SOUTH));
    pad.feed(&events[4..]);
    assert!(!pad.down(BTN_SOUTH) && pad.down(BTN_TR));
    assert_eq!(pad.trigger(0), 1.0);
    assert_eq!(pad.dpad(), (-1, 0));
    // Three reports 4 ms apart: two gaps, 250 a second.
    assert_eq!(pad.gaps, vec![4000, 4000]);
    assert_eq!(pad.rate(), Some(250.0));
    assert_eq!(pad.reports.len(), 3);
    assert_eq!(pad.reports[0].at_us, 1_000_000);
}

#[test]
fn an_overrun_drops_to_the_next_report() {
    let mut pad = xbox();
    let events = [
        Event::new(0, EV_ABS, ABS_X, 1000),
        Event::new(0, EV_SYN, SYN_REPORT, 0),
        Event::new(4000, EV_ABS, ABS_X, 9000),
        Event::new(4000, EV_SYN, SYN_DROPPED, 0),
        Event::new(4000, EV_ABS, ABS_X, 20000),
        Event::new(8000, EV_SYN, SYN_REPORT, 0),
        Event::new(12000, EV_ABS, ABS_Y, 500),
        Event::new(12000, EV_SYN, SYN_REPORT, 0),
    ];
    pad.feed(&events);
    // What came between the loss and the report after it was let go.
    assert_eq!(pad.axis(ABS_X).unwrap().value, 9000);
    assert_eq!(pad.axis(ABS_Y).unwrap().value, 500);
    assert!(!pad.dropped);
    // No gap is measured across the loss.
    assert!(pad.gaps.is_empty(), "gaps {:?}", pad.gaps);
}

#[test]
fn what_is_a_controller_and_where_its_parts_are() {
    let xpad = [ABS_X, ABS_Y, ABS_Z, ABS_RX, ABS_RY, ABS_RZ, ABS_HAT0X, ABS_HAT0Y];
    let buttons = [BTN_SOUTH, BTN_EAST, BTN_NORTH, BTN_WEST, BTN_TL, BTN_TR];
    assert!(is_controller(&buttons, &xpad, false));
    let l = Layout::of(&xpad, &buttons);
    assert_eq!((l.left, l.right, l.triggers, l.hat, l.numbered), (Some((ABS_X, ABS_Y)), Some((ABS_RX, ABS_RY)), [Some(ABS_Z), Some(ABS_RZ)], Some((ABS_HAT0X, ABS_HAT0Y)), false));
    // A Logitech F310 switched to D (DirectInput): Z and RZ are its right
    // stick, its buttons numbered from BTN_TRIGGER.
    let f310 = [ABS_X, ABS_Y, ABS_Z, ABS_RZ, ABS_HAT0X, ABS_HAT0Y];
    let numbered: Vec<u16> = (0x120..0x12c).collect();
    assert!(is_controller(&numbered, &f310, false));
    let l = Layout::of(&f310, &numbered);
    assert_eq!((l.right, l.triggers, l.numbered), (Some((ABS_Z, ABS_RZ)), [None, None], true));
    assert_eq!(button_name(0x120, false, true), "1");
    // A DualShock's touchpad and motion sensors, and a keyboard, are not.
    assert!(!is_controller(&[0x14a, 0x110], &[ABS_X, ABS_Y, ABS_MT_SLOT], false));
    assert!(!is_controller(&[], &[ABS_X, ABS_Y, ABS_Z, ABS_RX, ABS_RY, ABS_RZ], true));
    assert!(!is_controller(&[30, 31, 32], &[], false));
    // The face buttons named as each maker does.
    assert_eq!(button_name(BTN_NORTH, false, false), "X");
    assert_eq!(button_name(BTN_NORTH, true, false), "triangle");
    assert_eq!(button_name(BTN_MODE, true, false), "PS");
}

#[test]
fn axes_read_about_their_middle() {
    let a = |min, max, value| Axis { code: ABS_X, info: AbsInfo { min, max, ..AbsInfo::default() }, value };
    // xpad's sticks: −32768 to 32767, 0 is (a hair off) the middle.
    assert!(a(-32768, 32767, 0).centred().abs() < 1e-4);
    assert_eq!(a(-32768, 32767, 32767).centred(), 1.0);
    // A DualShock's: 0 to 255 about 128.
    assert!((a(0, 255, 128).centred() - 0.003_92).abs() < 1e-4);
    assert_eq!(a(0, 255, 0).centred(), -1.0);
    // Triggers from nothing to all.
    assert_eq!(a(0, 1023, 0).pulled(), 0.0);
    assert!((a(0, 255, 128).pulled() - 0.502).abs() < 0.001);
}

#[test]
fn drift_and_shake_at_rest_and_the_deadzone_for_them() {
    // Sitting at (0.05, −0.03), shaking ±0.01 in x: mean back exactly,
    // spread 0.01 (its root mean square), furthest √(0.06² + 0.03²).
    let samples: Vec<(f32, f32)> = (0..100).map(|i| (if i % 2 == 0 { 0.06 } else { 0.04 }, -0.03)).collect();
    let r = health::rest(&samples);
    assert!((r.drift.0 - 0.05).abs() < 1e-6 && (r.drift.1 + 0.03).abs() < 1e-6, "drift {:?}", r.drift);
    assert!((r.noise - 0.01).abs() < 1e-6, "noise {}", r.noise);
    assert!((r.furthest - 0.067_082).abs() < 1e-5);
    assert!((r.drift_len() - 0.058_31).abs() < 1e-5);
    // Its furthest and 0.02 to spare is 0.0871: 0.09.
    assert_eq!(health::deadzone(&r), 0.09);
    // A good stick still gets the least: 0.03.
    assert_eq!(health::deadzone(&health::rest(&[(0.002, 0.001); 10])), 0.03);
    // Exactly on a hundredth stays there.
    assert_eq!(health::deadzone(&Rest { furthest: 0.05, ..Rest::default() }), 0.07);
    assert_eq!(health::rest(&[]), Rest::default());
}

#[test]
fn range_reached_round_the_edge() {
    let circle = |r: f32| -> Vec<(f32, f32)> { (0..360).map(|d| (d as f32).to_radians()).map(|a| (r * a.cos(), r * a.sin())).collect() };
    let full = health::reach(&circle(0.98));
    assert!(full.iter().all(|r| (*r - 0.98).abs() < 1e-3), "{full:?}");
    assert!(health::full(&full));
    let worn = health::reach(&circle(0.9));
    assert!(!health::full(&worn));
    // Short only one way (a worn gate on the left).
    let mut one = circle(1.0);
    one.retain(|(x, _)| *x > -0.8);
    let r = health::reach(&one);
    assert!(r[1] < 0.81 && r[0] > 0.99, "{r:?}");
    // The sweep's sectors: east is the first, just below it the last.
    assert_eq!(health::sector(1.0, 0.0), Some(0));
    assert_eq!(health::sector(1.0, -0.01), Some(health::SECTORS - 1));
    assert_eq!(health::sector(0.5, 0.0), None, "not at the edge");
}

#[test]
fn how_often_reports_come() {
    // Every 4 ms, once 9 ms, and a 2 s pause (held still) that is not a gap.
    let mut times: Vec<i64> = (0..50).map(|i| i * 4000).collect();
    times.push(196_000 + 9000);
    times.push(205_000 + 2_000_000);
    times.push(205_000 + 2_004_000);
    let r = health::rate(&times).unwrap();
    assert_eq!(r.hz, 250.0);
    assert_eq!(r.median_ms, 4.0);
    assert_eq!(r.worst_ms, 9.0);
    assert!(health::rate(&[1, 2_000_000]).is_none());
}

#[test]
fn the_verdict() {
    let full = [[1.0; 4]; 2];
    let fast = Some(health::Rate { hz: 250.0, median_ms: 4.0, worst_ms: 6.0 });
    assert_eq!(health::verdict([0.05, 0.08], full, fast), Verdict::Good);
    assert_eq!(health::verdict([0.05, 0.09], full, fast), Verdict::Fair);
    assert_eq!(health::verdict([0.05, 0.16], full, fast), Verdict::Worn);
    assert_eq!(health::verdict([0.05, 0.05], [[1.0; 4], [0.9, 1.0, 1.0, 1.0]], fast), Verdict::Fair, "short of full throw");
    assert_eq!(health::verdict([0.05, 0.05], [[1.0; 4], [0.8, 1.0, 1.0, 1.0]], fast), Verdict::Worn);
    assert_eq!(health::verdict([0.05, 0.05], full, Some(health::Rate { hz: 60.0, median_ms: 16.7, worst_ms: 20.0 })), Verdict::Fair, "slow reports");
}

#[test]
fn a_check_on_the_simulated_controller() {
    crate::shell::tests::sandbox();
    let mut gp = Gamepad::with(Pads::simulated());
    gp.run_check(30.0);
    let rec = gp_record(&gp).expect("the check finished");
    // Its left stick sits at (0.046, −0.031) ± 0.006: drift 0.055, out to
    // 0.064 at most, so a deadzone of 0.09; the right one is worn short (0.9).
    assert!((rec.rest[0].drift_len() - 0.0555).abs() < 0.003, "{:?}", rec.rest[0]);
    assert_eq!(rec.deadzone[0], 0.09);
    assert!(health::full(&rec.reach[0]) && !health::full(&rec.reach[1]));
    assert_eq!(rec.rate.map(|r| r.hz), Some(250.0));
    assert_eq!(rec.verdict, Verdict::Fair);
    assert_eq!(rec.id, "045e:02ea:3039373132");
    // Kept and read back the same, best first among others.
    let mut rec = rec.clone();
    rec.label = "Driver".into();
    assert_eq!(Record::from_json(&rec.to_json()).as_ref(), Some(&rec));
    let path = keep(&rec).unwrap();
    assert!(path.ends_with("045e-02ea-3039373132-driver.json"));
    let mut better = rec.clone();
    better.label = "Spare".into();
    better.verdict = Verdict::Good;
    let other = keep(&better).unwrap();
    let all = kept();
    let order: Vec<&str> = all.iter().map(|(_, r)| r.label.as_str()).filter(|l| *l == "Driver" || *l == "Spare").collect();
    assert_eq!(order, ["Spare", "Driver"]);
    std::fs::remove_file(path).ok();
    std::fs::remove_file(other).ok();
}

fn gp_record(gp: &Gamepad) -> Option<Record> {
    gp.result().cloned()
}

/// At rest the page asks for nothing: the simulated controller's shake is
/// under half a pixel, so a minute of it draws nothing and the loop keeps
/// its idle look. Moved, it draws, but never twice in one beat.
#[test]
fn it_draws_only_what_changes_and_no_faster_than_the_beat() {
    crate::shell::tests::sandbox();
    let robot = crate::robot::Robot::none();
    let mut g = crate::gfx::Gfx::new(bezel::Tokens::dark());
    let mut gp = Gamepad::with(Pads::simulated());
    gp.draw(&mut g, &robot);
    let mut draws = 0;
    for _ in 0..3600 {
        if gp.ambient_tick(1.0 / 60.0, &robot) {
            draws += 1;
            gp.draw(&mut g, &robot);
        }
    }
    assert!(draws <= 2, "{draws} frames drawn at rest in a minute");
    assert_eq!(gp.ambient(), None, "at rest the loop keeps its own look");
    // Moving: as reports come every millisecond, a frame at most each 25 ms.
    // The last frame was a minute ago.
    let mut since = 60.0f32;
    let mut frames = 0;
    for i in 0..1000 {
        let v = (i as f32 * 0.01).sin();
        gp.pads_mut().hold([(v, 0.0), (0.0, 0.0)], [0.0, 0.0], &[], (0, 0));
        since += 0.001;
        if gp.ambient_tick(0.001, &robot) {
            assert!(since >= 1.0 / 40.0 - 1e-4, "drawn again after {since} s");
            since = 0.0;
            frames += 1;
            gp.draw(&mut g, &robot);
        }
    }
    assert!(frames >= 30, "only {frames} frames for a second of moving");
}
