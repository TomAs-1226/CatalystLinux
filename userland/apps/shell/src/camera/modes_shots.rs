//! Burst, light trails and the time-lapse in pictures
//! (`--camera-modes-shots DIR`, 1.5.1): each mode at rest with its chip,
//! its setting's choices, and as it is taken (the burst's ring counting,
//! the trails building on a street at night, a time-lapse running); the
//! eight modes' choices sliding; each mode's keys; a time-lapse in the
//! album, playing, and its details; dark and light at the three text sizes.

use super::bar::{Chip, Picker};
use super::lapse_mode::Taking;
use super::pixel_mode::{render, shot_camera};
use super::*;
use crate::shell::Shell;

/// A street at night as the camera sees it (`t` s along), its lights
/// moving, developed as the preview is; and its trails laid over `frames`
/// of it, as catalyst-camera lays them.
fn night_street(t: f32, frames: u32) -> (Picture, Picture) {
    let (w, h) = (PREVIEW_W, PREVIEW_H);
    let mut base = scene(w, h, t, false);
    sim_expose(&mut base, 0.16, 3600);
    let mut seed = 5u64;
    let frame = |i: u32, seed: &mut u64| {
        let mut pic = base.clone();
        trails::sim_lights(&mut pic, t + i as f32 / 15.0);
        let mut yuv = rgb_to_yuv420(&pic);
        sim_noise(&mut yuv, w * h, 3.0, seed);
        yuv
    };
    let develop = |yuv: &[u8]| {
        let mut out = Picture::default();
        Look::Original.compiled(0.0).develop(yuv, w, h, &mut out, None);
        out
    };
    let first = frame(0, &mut seed);
    let mut built = first.clone();
    for i in 1..frames {
        let f = frame(i, &mut seed);
        trails::lighten(&mut built, &f, w, h, trails::lighten_margin(6.0));
    }
    (develop(&first), develop(&built))
}

pub fn shots(dir: &std::path::Path) {
    crate::smartcam::sim::choose(false);
    crate::imu::simulate_standing(false);
    let day = scene(PREVIEW_W, PREVIEW_H, 3.1, false);
    let developed = |pic: &Picture| {
        let yuv = rgb_to_yuv420(pic);
        let mut out = Picture::default();
        Look::Original.compiled(0.0).develop(&yuv, PREVIEW_W, PREVIEW_H, &mut out, None);
        (out, pro::Histogram::of_luma(&yuv[..PREVIEW_W * PREVIEW_H], PREVIEW_W, PREVIEW_H, 2))
    };
    let (day_pic, day_hist) = developed(&day);
    let (street, trails_built) = night_street(2.0, 70);
    // A lit gym for the burst; an evening room; a street at night.
    let gym = Metered { exposure_us: 8_000, analogue: 1.0, digital: 1.0, gains: [1.9, 1.5], ct: 4800, lux: 900.0, settled: true, fom: 900, zoom: 0.0 };
    let room = Metered { exposure_us: 16_667, analogue: 2.0, digital: 1.0, gains: [1.82, 1.46], ct: 5200, lux: 320.0, settled: true, fom: 900, zoom: 0.0 };
    let night = Metered { exposure_us: 66_666, analogue: 8.0, digital: 1.0, gains: [1.45, 2.1], ct: 3400, lux: 6.0, settled: true, fom: 300, zoom: 0.0 };
    type Scene = (String, Box<dyn Fn(&mut Shell)>);
    type Setup = Box<dyn Fn(&mut Camera)>;
    let scene_of = |name: &str, mode: Mode, pic: Picture, hist: pro::Histogram, m: Metered, setup: Setup, keys: Vec<Key>| -> Scene {
        (
            name.to_owned(),
            Box::new(move |shell: &mut Shell| {
                let mut c = shot_camera();
                c.set_mode(mode);
                c.preview = Some(pic.clone());
                c.histogram = hist.clone();
                c.metered = Some(m);
                c.told = Some(pro::PreviewControls::auto(0.0, pro::PREVIEW_CAP_US));
                (c.burst_shutter, c.trails_s, c.lapse_every) = (burst_mode::SHUTTER_DEFAULT, trails::LENGTH_DEFAULT, crate::lapse::EVERY_DEFAULT);
                setup(&mut c);
                c.note = None;
                c.mode_at.jump(c.mode_at.target());
                shell.show(Box::new(c));
                for k in &keys {
                    for _ in 0..30 {
                        shell.tick(1.0 / 60.0);
                        shell.draw();
                    }
                    shell.key(*k);
                }
            }),
        )
    };
    let hist_of = |pic: &Picture| pro::Histogram::of_luma(&rgb_to_yuv420(pic)[..PREVIEW_W * PREVIEW_H], PREVIEW_W, PREVIEW_H, 2);
    let street_hist = hist_of(&street);
    // A burst under way: its ring, six of ten in.
    let burst_taking = |c: &mut Camera| {
        let plan = c.burst_ring();
        c.begin_stacking(camkit::Exposing::Burst, &plan);
        c.stack_news("6 10 4 520");
        if let Some(s) = &mut c.stacking {
            s.hold_for_shot(0.6);
        }
        c.saving = Some(Instant::now());
        c.captured = false;
        c.bar.hidden = true;
    };
    // Light trails building: 186 frames, 12.4 s of 30.
    let trails_running = |ended: bool| {
        move |c: &mut Camera| {
            let plan = c.trails_ring();
            c.begin_stacking(camkit::Exposing::Trails, &plan);
            c.stack_news("186 450 186 17600");
            if let Some(s) = &mut c.stacking {
                s.hold_for_shot(0.41);
                s.ended = ended;
            }
            c.saving = Some(Instant::now());
            c.captured = false;
            c.bar.hidden = true;
        }
    };
    // A time-lapse two minutes in, 24 frames kept, the next in three seconds.
    let lapse_running = |c: &mut Camera| {
        let now = Instant::now();
        let mut t = Taking::start(photos::dir().join("LAPSE_20261124_101500.avi"), 5, now - Duration::from_secs(120));
        (t.asked, t.saved, t.next) = (24, 24, now + Duration::from_secs(3));
        c.lapse = Some(t);
        c.bar.hidden = true;
    };
    // The saved burst's note (said as the photo lands).
    let saved = burst_mode::saved_words(4, 10);
    let burst_saved: Scene = ("06-burst-saved".into(), {
        let pic = day_pic.clone();
        let hist = day_hist.clone();
        Box::new(move |shell: &mut Shell| {
            let mut c = shot_camera();
            c.set_mode(Mode::Burst);
            c.preview = Some(pic.clone());
            c.histogram = hist.clone();
            c.metered = Some(gym);
            c.told = Some(pro::PreviewControls::auto(0.0, pro::PREVIEW_CAP_US));
            c.last = Some((Photo { path: PathBuf::from("IMG_burst.jpg"), name: "IMG_burst.jpg".into(), taken: chrono::Local::now().naive_local(), bytes: 0 }, pic.cover(photos::THUMB_W, photos::THUMB_H)));
            c.bar.hidden = true;
            c.mode_at.jump(c.mode_at.target());
            c.say(saved.clone());
            shell.show(Box::new(c));
        })
    });
    let mut scenes: Vec<Scene> = vec![
        scene_of("01-burst", Mode::Burst, day_pic.clone(), day_hist.clone(), gym, Box::new(|_| {}), Vec::new()),
        scene_of("02-burst-shutter-chip", Mode::Burst, day_pic.clone(), day_hist.clone(), gym, Box::new(|c| c.bar.focus = Some(Chip::Shutter)), Vec::new()),
        scene_of("03-burst-shutter-choices", Mode::Burst, day_pic.clone(), day_hist.clone(), gym, Box::new(|c| c.bar.picker = Some(Picker::Choice { chip: Chip::Shutter, at: 1 })), Vec::new()),
        scene_of("04-burst-in-low-light", Mode::Burst, day_pic.clone(), day_hist.clone(), room, Box::new(|c| c.bar.focus = Some(Chip::Shutter)), Vec::new()),
        scene_of("05-burst-taking", Mode::Burst, day_pic.clone(), day_hist.clone(), gym, Box::new(burst_taking), Vec::new()),
        burst_saved,
        scene_of("07-trails", Mode::Trails, street.clone(), street_hist.clone(), night, Box::new(|_| {}), Vec::new()),
        scene_of("08-trails-length-choices", Mode::Trails, street.clone(), street_hist.clone(), night, Box::new(|c| c.bar.picker = Some(Picker::Choice { chip: Chip::Length, at: 2 })), Vec::new()),
        scene_of("09-trails-building", Mode::Trails, trails_built.clone(), street_hist.clone(), night, Box::new(trails_running(false)), Vec::new()),
        scene_of("10-trails-finishing", Mode::Trails, trails_built.clone(), street_hist.clone(), night, Box::new(trails_running(true)), Vec::new()),
        scene_of("11-lapse", Mode::Lapse, day_pic.clone(), day_hist.clone(), room, Box::new(|_| {}), Vec::new()),
        scene_of("12-lapse-every-choices", Mode::Lapse, day_pic.clone(), day_hist.clone(), room, Box::new(|c| c.bar.picker = Some(Picker::Choice { chip: Chip::Every, at: 1 })), Vec::new()),
        scene_of("13-lapse-running", Mode::Lapse, day_pic.clone(), day_hist.clone(), room, Box::new(lapse_running), Vec::new()),
        scene_of("14-mode-choices", Mode::Trails, street.clone(), street_hist.clone(), night, Box::new(|c| c.bar.picker = Some(Picker::Choice { chip: Chip::Mode, at: 4 })), Vec::new()),
        scene_of("15-mode-choices-end", Mode::Doc, day_pic.clone(), day_hist.clone(), room, Box::new(|c| c.bar.picker = Some(Picker::Choice { chip: Chip::Mode, at: 7 })), Vec::new()),
        scene_of("16-keys-burst", Mode::Burst, day_pic.clone(), day_hist.clone(), gym, Box::new(|_| {}), vec![Key::Char('?')]),
        scene_of("17-keys-trails", Mode::Trails, street.clone(), street_hist.clone(), night, Box::new(|_| {}), vec![Key::Char('?')]),
        scene_of("18-keys-lapse", Mode::Lapse, day_pic.clone(), day_hist.clone(), room, Box::new(|_| {}), vec![Key::Char('?')]),
        scene_of("19-keys-lapse-running", Mode::Lapse, day_pic.clone(), day_hist.clone(), room, Box::new(lapse_running), vec![Key::Char('?')]),
    ];
    // A time-lapse kept: its frames made into one, in the album.
    let lapse_path = photos::dir().join("LAPSE_20261124_101500.avi");
    {
        let work = crate::lapse::work_of(&lapse_path);
        std::fs::create_dir_all(&work).ok();
        let (fw, fh) = (crate::lapse::FRAME.0 as usize, crate::lapse::FRAME.1 as usize);
        for i in 0..16u32 {
            let mut pic = scene(fw, fh, 1.0 + i as f32 * 0.6, false);
            sim_expose(&mut pic, 0.9 + 0.02 * i as f32, 5200);
            crate::burst::write_jpeg(&crate::lapse::frame_path(&work, i + 1), &rgb_to_yuv420(&pic), fw, fh, None).ok();
        }
        crate::lapse::assemble(&lapse_path).ok();
        photos::changed();
        if let Some(photo) = photos::list().into_iter().find(|p| p.path == lapse_path) {
            photos::thumbnail(&photo);
        }
    }
    {
        let lapse_path = lapse_path.clone();
        scenes.push((
            "20-album-with-a-time-lapse".into(),
            Box::new(move |shell: &mut Shell| {
                shell.show(Box::new(photos::Album::at(&lapse_path)));
                for _ in 0..40 {
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                    std::thread::sleep(Duration::from_millis(5));
                }
            }),
        ));
    }
    let player = |details: bool| -> Box<dyn Fn(&mut Shell)> {
        let lapse_path = lapse_path.clone();
        Box::new(move |shell: &mut Shell| {
            shell.show(Box::new(photos::Viewer::single(lapse_path.clone())));
            for _ in 0..60 {
                shell.tick(1.0 / 60.0);
                shell.draw();
                std::thread::sleep(Duration::from_millis(8));
            }
            if details {
                shell.key(Key::Char('i'));
            }
        })
    };
    scenes.push(("21-time-lapse-playing".into(), player(false)));
    scenes.push(("22-time-lapse-details".into(), player(true)));
    render(dir, &scenes);
    std::fs::remove_file(&lapse_path).ok();
    crate::imu::simulate_standing(false);
    println!("burst, light trails and the time-lapse in {}", dir.display());
}
