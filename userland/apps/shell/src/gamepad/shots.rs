//! Gamepad's pages on a simulated controller, dark and light, at each text
//! size (`--gamepad-shots <dir>`): `<n>-<page>-<dark|light>-<size>.png`.

use std::path::Path;

use super::evdev::*;
use super::health::{Rate, Rest, Verdict};
use super::pads::Pads;
use super::{Gamepad, Record, Usb};
use crate::gfx::TextSize;
use crate::shell::{Screen, Shell};

/// A DualSense as hid-playstation gives it.
fn dualsense() -> Pad {
    let stick = AbsInfo { value: 128, min: 0, max: 255, fuzz: 0, flat: 0 };
    let trigger = AbsInfo { value: 0, min: 0, max: 255, fuzz: 0, flat: 0 };
    let hat = AbsInfo { value: 0, min: -1, max: 1, fuzz: 0, flat: 0 };
    let axes = vec![(ABS_X, stick), (ABS_Y, stick), (ABS_Z, trigger), (ABS_RX, stick), (ABS_RY, stick), (ABS_RZ, trigger), (ABS_HAT0X, hat), (ABS_HAT0Y, hat)];
    let keys = vec![BTN_SOUTH, BTN_EAST, BTN_NORTH, BTN_WEST, BTN_TL, BTN_TR, BTN_TL2, BTN_TR2, BTN_SELECT, BTN_START, BTN_MODE, BTN_THUMBL, BTN_THUMBR];
    Pad::new("event10", "Sony Interactive Entertainment DualSense Wireless Controller", Ident { bus: 3, vendor: SONY, product: 0x0ce6, version: 0x8111 }, "a0:ab:51:3c:22:10", axes, keys, true)
}

fn record(label: &str, model: &str, id: &str, dz: [f32; 2], reach: [f32; 2], verdict: Verdict, days_ago: i64) -> Record {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let rest = |d: f32| Rest { drift: (d * 0.6, -d * 0.4), furthest: d, noise: 0.003 };
    Record {
        id: id.into(),
        label: label.into(),
        model: model.into(),
        when: now - days_ago * 86_400,
        rest: [rest(dz[0] - 0.02), rest(dz[1] - 0.02)],
        deadzone: dz,
        reach: [[reach[0]; 4], [reach[1]; 4]],
        rate: Some(Rate { hz: 250.0, median_ms: 4.0, worst_ms: 9.0 }),
        verdict,
    }
}

pub fn shots(dir: &Path) {
    std::fs::create_dir_all(dir).expect("the shots folder");
    let state = std::env::temp_dir().join(format!("catalyst-gamepad-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&state).ok();
    std::fs::create_dir_all(&state).expect("a state folder");
    std::env::set_var("CATALYST_CONF", state.join("robot"));
    for r in [
        record("Driver", "Microsoft X-Box One S pad", "045e:02ea:3039373132", [0.07, 0.05], [1.0, 0.99], Verdict::Good, 0),
        record("Spare 1", "Logitech Gamepad F310", "046d:c21d", [0.11, 0.06], [0.97, 0.98], Verdict::Fair, 3),
        record("Practice pad", "Sony DualShock 4", "054c:09cc:a0ab513c2210", [0.19, 0.08], [0.82, 0.97], Verdict::Worn, 12),
    ] {
        super::keep(&r).expect("kept");
    }
    let xbox_pose = |gp: &mut Gamepad| gp.pads_mut().hold([(0.55, -0.42), (-0.25, 0.6)], [0.62, 0.0], &[BTN_SOUTH, BTN_TR, BTN_THUMBR], (1, 0));
    let pages: Vec<(&str, Box<dyn Fn() -> Box<dyn Screen>>)> = vec![
        (
            "1-live-xbox",
            Box::new(move || {
                let mut gp = Gamepad::with(Pads::simulated());
                xbox_pose(&mut gp);
                Box::new(gp)
            }),
        ),
        (
            "2-live-playstation",
            Box::new(|| {
                let mut pads = Pads::simulated();
                pads.list[0] = dualsense();
                let mut gp = Gamepad::with(pads);
                gp.pads_mut().hold([(-0.7, 0.1), (0.0, -0.85)], [0.0, 0.9], &[BTN_NORTH, BTN_WEST, BTN_TL, BTN_THUMBL, BTN_TR2], (0, -1));
                Box::new(gp)
            }),
        ),
        (
            "3-every-input",
            Box::new(move || {
                let mut gp = Gamepad::with(Pads::simulated()).on_page(1);
                xbox_pose(&mut gp);
                Box::new(gp)
            }),
        ),
        (
            "4-check-hands-off",
            Box::new(|| {
                let mut gp = Gamepad::with(Pads::simulated()).on_page(2);
                gp.run_check(0.2);
                Box::new(gp)
            }),
        ),
        (
            "5-check-sweep",
            Box::new(|| {
                let mut gp = Gamepad::with(Pads::simulated()).on_page(2);
                gp.run_check(2.1);
                Box::new(gp)
            }),
        ),
        (
            "6-check-result",
            Box::new(|| {
                let mut gp = Gamepad::with(Pads::simulated()).on_page(2);
                gp.run_check(30.0);
                Box::new(gp)
            }),
        ),
        (
            "7-keep-as",
            Box::new(|| {
                let mut gp = Gamepad::with(Pads::simulated()).on_page(2);
                gp.run_check(30.0);
                Box::new(gp.naming("Driver"))
            }),
        ),
        ("8-kept", Box::new(|| Box::new(Gamepad::with(Pads::simulated()).on_page(3)))),
        ("9-unplugged", Box::new(|| Box::new(Gamepad::with(Pads::none())))),
        (
            "10-usb-for-a-computer",
            Box::new(|| {
                super::demo_usb(Some(Usb::ComputerMode));
                Box::new(Gamepad::with(Pads::none()))
            }),
        ),
    ];
    for light in [false, true] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            for (name, page) in &pages {
                let mut shell: Shell = crate::fullheight::shell("catalyst", size);
                shell.settings.light = light;
                shell.apply_settings();
                crate::fullheight::settle(&mut shell, 30);
                shell.show(page());
                crate::fullheight::settle(&mut shell, 90);
                shell.draw();
                let file = format!("{name}-{}-{}.png", if light { "light" } else { "dark" }, size.name());
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(file), 2);
                super::demo_usb(None);
            }
        }
    }
    std::fs::remove_dir_all(&state).ok();
    println!("Gamepad's pages in {}", dir.display());
}
