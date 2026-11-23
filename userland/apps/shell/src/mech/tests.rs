//! Mech's numbers against ones worked by hand (Python, from the formulas in
//! model.rs's header and ReCalc's, app/lib/models/Motor.ts and
//! app/lib/math/linear.ts at a1cde22), each within the tolerance it states.

use super::model::{self, motor, Stage, StageKind};
use super::*;

/// Within `pct` percent of `want`.
fn near(got: f64, want: f64, pct: f64) -> bool {
    (got - want).abs() <= want.abs() * pct / 100.0
}

macro_rules! close {
    ($got:expr, $want:expr, $pct:expr) => {{
        let (got, want) = ($got, $want);
        assert!(near(got, want, $pct), "{} = {got}, want {want} ± {}%", stringify!($got), $pct);
    }};
}

fn kraken() -> &'static model::Motor {
    motor("Kraken X60").expect("the Kraken X60 is in the table")
}

#[test]
fn the_motor_table_reads_whole_and_names_its_sources() {
    let all = model::motors();
    assert_eq!(all.len(), 16, "every motor line reads");
    for name in ["Kraken X60", "Kraken X60 FOC", "Kraken X44", "NEO", "NEO Vortex", "NEO 550", "Falcon 500", "CIM", "mini-CIM"] {
        assert!(motor(name).is_some(), "{name} is in the table");
    }
    for m in all {
        assert!(["CTRE", "VEX", "REV", "Thrifty"].contains(&m.source.as_str()), "{} has a source: {}", m.name, m.source);
        assert!(m.stall_a > m.free_a && m.free_rpm > 0.0 && m.stall_nm > 0.0, "{} is sane", m.name);
    }
    // A malformed line is left out rather than read as zeros.
    assert_eq!(model::parse_table("# c\nGood\t100\t1\t10\t1\tVEX\nBad\t100\tx\t10\t1\tVEX\n").len(), 1);
}

/// ReCalc's constants for the Kraken X60: R = V/I_stall, kV, kT.
#[test]
fn motor_constants_match_recalc() {
    let m = kraken();
    close!(m.resistance(), 0.032_052_74, 0.01);
    close!(m.kv(), 53.330_24, 0.01);
    close!(m.kt(), 0.019_262_39, 0.01);
    // The ends of the line: all the torque at rest, none at free speed.
    close!(m.torque_at(0.0, 1000.0), 7.157, 0.01);
    assert!(m.torque_at(m.free_rad(), 1000.0).abs() < 1e-9);
    close!(m.current_at(m.free_rad(), 1000.0), 2.83, 0.01);
}

#[test]
fn a_train_multiplies_its_stages() {
    let stages = [Stage { kind: StageKind::Gear, driving: 12, driven: 60 }, Stage { kind: StageKind::Belt, driving: 18, driven: 36 }];
    assert!((model::reduction(&stages) - 10.0).abs() < 1e-12);
    let o = model::ratio(kraken(), 1.0, 10.0, 1.0, 60.0);
    close!(o.free_rpm, 606.5, 0.01);
    close!(o.stall_nm, 71.57, 0.01);
    // kT × (60 − 2.83) × 10 = 11.012 N·m at a 60 A limit.
    close!(o.limited_nm, 11.012_31, 0.01);
    // Two motors, 90 % through the train: twice the torque, less 10 %.
    close!(model::ratio(kraken(), 2.0, 10.0, 0.9, 1000.0).stall_nm, 128.826, 0.01);
}

/// ReCalc's linear calculator, `calculateStallLoad`: two Kraken X60s at a
/// 40 A stator limit, 5:1, on a 1.5 in spool, 100 %: 84.49 lb.
#[test]
fn elevator_stall_load_matches_recalc() {
    let e = model::Elevator { motors: 2.0, ratio: 5.0, spool_in: 1.5, load_lb: 15.0, travel_in: 48.0, limit: 40.0, efficiency: 1.0 };
    let o = model::elevator(kraken(), &e);
    close!(o.max_lb, 84.493, 0.1);
    // 15 lb on that: 9.43 A a motor, rising at 7.80 ft/s; free 7.94 ft/s.
    close!(o.hold_a, 9.4288, 0.1);
    close!(o.loaded_fps, 7.7981, 0.1);
    close!(o.free_fps, 7.9391, 0.1);
    assert!(o.time_s.is_some_and(|t| t > 4.0 / 7.94), "four feet takes at least four feet at free speed");
}

#[test]
fn arm_holding_current_at_its_worst_angle() {
    // 10 lb at 12 in: 10 lbf·ft = 13.558 N·m at horizontal; through 100:1
    // on one Kraken X60 that is 0.13558 N·m, 7.04 A over its 2.83 A free.
    let a = model::Arm { motors: 1.0, ratio: 100.0, length_in: 24.0, mass_lb: 10.0, com_in: 12.0, start_deg: 0.0, end_deg: 90.0, limit: 40.0, efficiency: 1.0 };
    let o = model::arm(kraken(), &a);
    close!(o.gravity_nm, 13.558_18, 0.01);
    close!(o.hold_a, 9.868_68, 0.05);
    assert_eq!(o.worst_deg, 0.0, "horizontal is the worst");
    // A move that never passes horizontal is worst at its lower end.
    let o = model::arm(kraken(), &model::Arm { start_deg: 30.0, end_deg: 80.0, ..a });
    assert_eq!(o.worst_deg, 30.0);
    close!(o.gravity_nm, 13.558_18 * 30f64.to_radians().cos(), 0.01);
    // Through the top it is horizontal on the far side.
    assert_eq!(model::arm(kraken(), &model::Arm { start_deg: 45.0, end_deg: 200.0, ..a }).worst_deg, 180.0);
    let t = model::arm(kraken(), &a).time_s.expect("it lifts it");
    assert!(t > 0.05 && t < 2.0, "a quarter turn at 100:1 takes {t} s");
    // Too heavy to lift: it never gets there.
    assert!(model::arm(kraken(), &model::Arm { mass_lb: 400.0, ..a }).time_s.is_none());
}

/// No current limit: ω(t) = ω_free(1 − e^(−t/τ)), τ = J·ω_free/τ_stall,
/// so spin-up to ω is −τ·ln(1 − ω/ω_free). 5 lb·in² on one Kraken X60 at
/// 1:1 to 4000 rpm: τ = 0.12985 s, 0.13990 s to get there; back from a
/// 10 % drop, τ·ln((ω_f − 0.9ω)/(ω_f − ω)) = 0.02299 s.
#[test]
fn flywheel_spin_up_matches_the_closed_form() {
    let f = model::Flywheel { motors: 1.0, ratio: 1.0, moi_lb_in2: 5.0, target_rpm: 4000.0, drop: 0.1, limit: 1000.0, efficiency: 1.0 };
    let o = model::flywheel(kraken(), &f);
    close!(o.spin_up_s.unwrap(), 0.139_897_6, 0.2);
    close!(o.recover_s.unwrap(), 0.022_990_9, 0.5);
    close!(o.energy_j, 0.5 * 5.0 * model::LB_IN2 * (4000.0f64 * std::f64::consts::TAU / 60.0).powi(2), 0.01);
    // Past free speed it never gets there.
    assert!(model::flywheel(kraken(), &model::Flywheel { target_rpm: 7000.0, ..f }).spin_up_s.is_none());
    // A current limit only slows it.
    assert!(model::flywheel(kraken(), &model::Flywheel { limit: 40.0, ..f }).spin_up_s.unwrap() > 0.1399);
}

#[test]
fn drivetrain_speed_push_and_slip() {
    // Two Kraken X60s a side, 6.75:1, 4 in wheels: 15.68 ft/s free; 81 %
    // of it is 12.70. 140 lb at μ 1.1 pushes 154 lbf before the wheels
    // slip, at 77.2 A a motor through 90 %.
    let d = model::Drive { per_side: 2.0, ratio: 6.75, wheel_in: 4.0, mass_lb: 140.0, mu: 1.1, limit: 60.0, efficiency: 0.9, speed_loss: 0.81, distance_ft: 15.0 };
    let o = model::drive(kraken(), &d);
    close!(o.free_fps, 15.682, 0.01);
    close!(o.top_fps, 12.7025, 0.01);
    close!(o.slip_a, 77.175, 0.05);
    // At 60 A the motors give 118.4 lbf, under the grip: the limit holds it.
    close!(o.push_lbf, 118.423, 0.05);
    assert!(!o.traction_limited);
    close!(o.accel_fps2, 27.215, 0.05);
    // With no limit the wheels slip first, and push with all the grip.
    let o = model::drive(kraken(), &model::Drive { limit: 1000.0, ..d });
    assert!(o.traction_limited);
    close!(o.push_lbf, 154.0, 0.01);

    // Grip and limit out of the way, with no losses, it is the motor alone:
    // v = v_f(1 − e^(−t/τ)), x = v_f(t − τ(1 − e^(−t/τ))), τ = m·v_f/F_stall.
    let pure = model::Drive { mu: 100.0, limit: 1000.0, efficiency: 1.0, speed_loss: 1.0, ..d };
    let o = model::drive(kraken(), &pure);
    let m = kraken();
    let r = 2.0 * model::INCH;
    let vf = m.free_rad() / 6.75 * r;
    let f0 = 4.0 * m.stall_nm * 6.75 / r;
    let tau = 140.0 * model::POUND * vf / f0;
    let x = |t: f64| vf * (t - tau * (1.0 - (-t / tau).exp()));
    let (mut lo, mut hi) = (0.0, 10.0);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if x(mid) < 15.0 * model::FOOT {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    close!(o.time_s.unwrap(), lo, 0.2);
}

#[test]
fn fields_take_only_what_fits() {
    let f = Calc::Drive.fields();
    let ratio = f.iter().find(|f| f.key == "ratio").unwrap();
    assert_eq!(ratio.accept(-3.0), None);
    assert_eq!(ratio.accept(0.0), None);
    assert_eq!(ratio.show(6.75), "6.75:1");
    let n = f.iter().find(|f| f.key == "per_side").unwrap();
    assert_eq!(n.accept(2.6), Some(3.0));
    assert_eq!(n.accept(40.0), Some(12.0));
    let pct = f.iter().find(|f| f.key == "efficiency").unwrap();
    assert_eq!(pct.accept(140.0), Some(100.0));
    assert_eq!(pct.show(90.0), "90%");
    let angle = Calc::Arm.fields().iter().find(|f| f.key == "start").unwrap();
    assert_eq!(angle.accept(-30.0), Some(-30.0));
    assert_eq!(parse_teeth("12:60"), Some((12, 60)));
    assert_eq!(parse_teeth("12:0"), None);
    assert_eq!(parse_teeth("60"), None);
    assert_eq!(parse_stage_line("chain 16:32").map(|s| s.kind), Some(StageKind::Chain));
}

#[test]
fn a_kept_mechanism_reads_back_the_same() {
    let mut m = Mech::new(Calc::Ratio);
    m.name = "Wrist".into();
    m.motor = 7;
    m.values[1] = 2.0;
    m.stages[2] = Some(Stage { kind: StageKind::Chain, driving: 16, driven: 32 });
    assert_eq!(Mech::from_json(&m.to_json()), Some(m));
    let mut a = Mech::new(Calc::Arm);
    a.name = "Pivot".into();
    a.values[6] = -20.0;
    assert_eq!(Mech::from_json(&a.to_json()), Some(a));
    assert_eq!(slug(" Shooter / 2026! "), "shooter-2026");
    assert_eq!(slug("??"), "mechanism");
}

#[test]
fn kept_in_the_mech_folder_and_listed_by_name() {
    crate::shell::tests::sandbox();
    let mut m = Mech::new(Calc::Flywheel);
    m.name = "Shooter test".into();
    let path = save(&m).expect("kept");
    assert!(path.starts_with(dir()) && path.ends_with("shooter-test.json"));
    assert!(saved().iter().any(|(_, k)| *k == m));
    std::fs::remove_file(path).ok();
}

#[test]
fn every_calculator_gives_results_and_a_line() {
    for c in Calc::ALL {
        let s = Mech::new(c).solve();
        assert!(s.plot.line.len() >= 2, "{} draws a line", c.name());
        assert!(s.rows.iter().all(|(_, v)| !v.contains("NaN") && !v.contains("inf")), "{} reads cleanly: {:?}", c.name(), s.rows);
        assert!(s.heads.iter().all(|(_, v, _)| v != "–"), "{} works out with its defaults: {:?}", c.name(), s.rows);
    }
}

#[test]
fn axis_ends_are_round_numbers() {
    use super::screen::nice;
    assert_eq!(nice(606.5), 1000.0);
    assert_eq!(nice(71.57), 100.0);
    assert_eq!(nice(15.7), 20.0);
    assert_eq!(nice(2.3), 2.5);
    assert_eq!(nice(0.14), 0.2);
    assert_eq!(nice(5.0), 5.0);
}
