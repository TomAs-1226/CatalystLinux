//! The AprilTag reader: the camera's picture with every 36h11 tag in it
//! outlined and named, and for each, how far away it is, how far to the side,
//! and how far it is turned: a check of what the robot's vision should see.
//!
//! On the handheld the picture comes from the IMX219 through libcamera
//! (`catalyst-camera`, the image's own small program, started when the reader
//! opens and stopped when it closes; `rpicam-vid` where a system has that
//! instead), 640 × 480 at 15 frames a second from the sensor's full-view
//! binned mode,
//! so the lens is known: nominally 530 pixels of focal length, or a
//! calibration in `apriltag/lens.json` beside the settings (`{"fx": …, "fy": …,
//! "cx": …, "cy": …}`, as PhotonVision's calibration gives them). On a
//! computer, a simulated camera shows a tag moving in front of it, read by the
//! same detector.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use apriltag::{Detector, Lens, Tag, FRC_TAG_SIZE};
use bezel::{Align, Color, Rect};

use crate::gfx::{Gfx, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// The picture read: 640 × 480, as the camera gives it.
const CAM_W: usize = 640;
const CAM_H: usize = 480;
/// The preview kept for the screen: a quarter each way.
const PREVIEW_W: usize = 160;
const PREVIEW_H: usize = 120;

/// The IMX219's lens at 640 × 480 from its full-view 2 × 2 binned mode:
/// 3.04 mm over 2.24 µm binned pixels is 1357 pixels at 1640 across, so 530
/// at 640. Nominal, until calibrated.
pub const NOMINAL: Lens = Lens { fx: 529.6, fy: 529.6, cx: 320.0, cy: 240.0 };

/// What the reader thread last made of the camera.
#[derive(Default)]
struct View {
    preview: Vec<u8>,
    tags: Vec<Tag>,
    frames: u64,
    fps: f32,
    /// What is wrong, if the camera cannot be read.
    trouble: Option<String>,
}

/// Where the lens comes from: a calibration file, or the nominal one.
fn lens() -> (Lens, bool) {
    let file = crate::config_dir().join("apriltag").join("lens.json");
    let read = std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| {
        let f = |k: &str| v.get(k).and_then(serde_json::Value::as_f64).filter(|x| x.is_finite() && *x > 0.0);
        Some(Lens { fx: f("fx")?, fy: f("fy")?, cx: f("cx")?, cy: f("cy")? })
    });
    match read {
        Some(lens) => (lens, true),
        None => (NOMINAL, false),
    }
}

/// A picture source: the camera, or the simulated one.
enum Source {
    /// A camera program's output: each frame the brightness plane, then (for
    /// a YUV 4:2:0 frame from rpicam-vid) `colour` bytes more to pass over.
    Camera { child: std::process::Child, stdout: std::process::ChildStdout, colour: usize },
    Simulated { started: Instant, scratch: Vec<u8> },
}

/// libcamera's Raspberry Pi pipeline needs the firmware's image processor
/// (the ISP) as well as the sensor, and the ISP is reached through vchiq,
/// which the image does not load at start (nothing else needs it, and it
/// holds memory): without it the sensor is found but libcamera reports no
/// camera. Loaded here the first time the reader opens, then kept.
fn ready_the_isp() {
    let has_isp = || {
        std::fs::read_dir("/sys/class/video4linux")
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| std::fs::read_to_string(e.path().join("name")).is_ok_and(|n| n.contains("isp")))
    };
    if has_isp() {
        return;
    }
    // By name: the blacklist only keeps them from loading by themselves.
    std::process::Command::new("modprobe").args(["-a", "vchiq", "vc-sm-cma", "bcm2835-isp"]).status().ok();
    // The ISP's video devices appear as vchiq finds the firmware's services.
    for _ in 0..40 {
        if has_isp() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

impl Source {
    /// The camera on the handheld; on a computer, the simulated one.
    fn open() -> Result<Source, String> {
        if !cfg!(target_arch = "aarch64") {
            return Ok(Source::Simulated { started: Instant::now(), scratch: Vec::new() });
        }
        ready_the_isp();
        // 640 × 480 frames on stdout, from the sensor's full-view binned mode,
        // until stopped: catalyst-camera sends the brightness alone;
        // rpicam-vid (no preview window) whole YUV 4:2:0 frames.
        let ours: &[&str] = &["--width", "640", "--height", "480", "--fps", "15"];
        let rpicam: &[&str] = &["-t", "0", "-n", "--codec", "yuv420", "--width", "640", "--height", "480", "--framerate", "15", "--mode", "1640:1232:10:P", "--flush", "-o", "-"];
        let programs = [("catalyst-camera", ours, 0), ("rpicam-vid", rpicam, CAM_W * CAM_H / 2), ("libcamera-vid", rpicam, CAM_W * CAM_H / 2)];
        for (program, args, colour) in programs {
            // libcamera's own log only for what goes wrong: its standard
            // error is read only once the picture stops.
            let mut command = std::process::Command::new(program);
            command.args(args).env("LIBCAMERA_LOG_LEVELS", "*:WARN");
            match command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn() {
                Ok(mut child) => {
                    let stdout = child.stdout.take().ok_or("no picture from the camera")?;
                    return Ok(Source::Camera { child, stdout, colour });
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("the camera would not start: {e}")),
            }
        }
        Err("no camera program in this system".into())
    }

    /// The next picture's brightness, 640 × 480, into `y`.
    fn next(&mut self, y: &mut Vec<u8>) -> Result<(), String> {
        match self {
            Source::Camera { child, stdout, colour } => {
                y.resize(CAM_W * CAM_H, 0);
                let mut rest = vec![0u8; *colour];
                if stdout.read_exact(y).and_then(|()| stdout.read_exact(&mut rest)).is_err() {
                    let mut why = String::new();
                    if let Some(err) = child.stderr.as_mut() {
                        err.read_to_string(&mut why).ok();
                    }
                    let last = why.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("the camera stopped").trim().to_lowercase();
                    return Err(if last.contains("no cameras available") { "no camera found".into() } else { last });
                }
                Ok(())
            }
            Source::Simulated { started, scratch } => {
                // At the camera's pace.
                std::thread::sleep(Duration::from_millis(66));
                simulate(started.elapsed().as_secs_f32(), y, scratch);
                Ok(())
            }
        }
    }

    fn stop(&mut self) {
        if let Source::Camera { child, .. } = self {
            child.kill().ok();
            child.wait().ok();
        }
    }
}

/// The AprilTag reader.
pub struct Reader {
    view: Arc<Mutex<View>>,
    stop: Arc<AtomicBool>,
    seen: u64,
    paused: bool,
    /// What is on screen: the view as last taken, unless paused.
    shown: View,
    calibrated: bool,
    simulated: bool,
}

impl Reader {
    pub fn new() -> Self {
        let view = Arc::new(Mutex::new(View::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (lens, calibrated) = lens();
        let simulated = !cfg!(target_arch = "aarch64");
        let (v, s) = (view.clone(), stop.clone());
        std::thread::Builder::new()
            .name("apriltags".into())
            .spawn(move || read(v, s, lens))
            .ok();
        Reader { view, stop, seen: 0, paused: false, shown: View::default(), calibrated, simulated }
    }
}

/// The reader's thread: pictures in, tags found, a preview kept.
fn read(view: Arc<Mutex<View>>, stop: Arc<AtomicBool>, lens: Lens) {
    let mut source = match Source::open() {
        Ok(s) => s,
        Err(why) => {
            view.lock().map(|mut v| v.trouble = Some(why)).ok();
            return;
        }
    };
    // Two of the four cores, so the shell stays smooth while it looks.
    let mut detector = Detector::new(2, 2.0);
    let mut y = Vec::new();
    let mut preview = vec![0u8; PREVIEW_W * PREVIEW_H];
    let mut last = Instant::now();
    let mut fps = 0.0f32;
    while !stop.load(Ordering::Relaxed) {
        if let Err(why) = source.next(&mut y) {
            view.lock().map(|mut v| v.trouble = Some(why)).ok();
            break;
        }
        let tags = detector.detect(&y, CAM_W, CAM_H, FRC_TAG_SIZE, Some(lens));
        // A quarter each way, averaged.
        for py in 0..PREVIEW_H {
            for px in 0..PREVIEW_W {
                let mut sum = 0u32;
                for dy in 0..4 {
                    let row = (py * 4 + dy) * CAM_W + px * 4;
                    sum += y[row..row + 4].iter().map(|&b| b as u32).sum::<u32>();
                }
                preview[py * PREVIEW_W + px] = (sum / 16) as u8;
            }
        }
        let now = Instant::now();
        let gap = now.duration_since(last).as_secs_f32().max(1e-3);
        last = now;
        fps = if fps == 0.0 { 1.0 / gap } else { fps * 0.85 + 0.15 / gap };
        if let Ok(mut v) = view.lock() {
            v.preview.clone_from(&preview);
            v.tags = tags;
            v.frames += 1;
            v.fps = fps;
            v.trouble = None;
        }
    }
    source.stop();
}

impl Drop for Reader {
    fn drop(&mut self) {
        // The thread stops the camera when it sees this (at its next picture).
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Screen for Reader {
    fn title(&self) -> &str {
        "AprilTag"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => Nav::Pop,
            Key::Char(' ') | Key::Enter => {
                self.paused = !self.paused;
                *cx.say = Some(if self.paused { "held".into() } else { "live".into() });
                Nav::Stay
            }
            _ => Nav::Stay,
        }
    }

    fn busy(&self) -> bool {
        !self.paused
    }

    /// While the picture is live, someone is aiming the camera.
    fn keeps_awake(&self) -> bool {
        !self.paused
    }

    /// Not kept among recent apps: leaving it stops the camera, which would
    /// otherwise go on running unseen.
    fn keep_in_recents(&self) -> bool {
        false
    }

    fn tick(&mut self, _dt: f32) -> bool {
        if self.paused {
            return false;
        }
        let Ok(mut v) = self.view.lock() else { return false };
        if v.frames == self.seen && v.trouble == self.shown.trouble {
            return false;
        }
        self.seen = v.frames;
        self.shown.preview = std::mem::take(&mut v.preview);
        self.shown.tags.clone_from(&v.tags);
        (self.shown.frames, self.shown.fps, self.shown.trouble) = (v.frames, v.fps, v.trouble.clone());
        true
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (frame, side) = layout(g);
        g.surface(frame.inset(-2.0), g.radius(8.0), t.surface1);
        if self.shown.preview.len() == PREVIEW_W * PREVIEW_H {
            draw_preview(g, frame, &self.shown.preview);
        } else {
            let words = self.shown.trouble.clone().unwrap_or_else(|| "starting the camera".into());
            let words = g.text.fit(&words, &ty.caption, frame.w - 12.0);
            g.text_at(frame.cx(), frame.cy(), Align::Center, &words, ty.caption, t.dim);
        }
        // Each tag outlined where it is, its number just above it (so the
        // tag itself stays in sight, however small it is in the picture).
        let (sx, sy) = (frame.w / CAM_W as f32, frame.h / CAM_H as f32);
        let ring = g.ring_color();
        g.canvas.set_clip(frame);
        for tag in &self.shown.tags {
            let pts: Vec<(f32, f32)> = tag.corners.iter().map(|(x, y)| (frame.x + x * sx, frame.y + y * sy)).collect();
            for i in 0..4 {
                line(g, pts[i], pts[(i + 1) % 4], 1.6, ring);
            }
            let label = tag.id.to_string();
            let w = g.measure(&label, ty.caption) + 8.0;
            let cx = frame.x + tag.center.0 * sx;
            let above = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min);
            let cy = if above - 10.0 > frame.y + 2.0 { above - 9.0 } else { pts.iter().map(|p| p.1).fold(f32::MIN, f32::max) + 9.0 };
            let chip = Rect::new((cx - w * 0.5).clamp(frame.x, frame.right() - w), cy - 7.0, w, 14.0);
            g.canvas.rrect(chip, 7.0, t.signal, 1.0);
            g.text_at(chip.cx(), chip.cy(), Align::Center, &label, ty.caption, t.on_signal);
        }
        g.canvas.clear_clip();

        // Beside it: each tag, nearest first.
        let mut tags: Vec<&Tag> = self.shown.tags.iter().collect();
        tags.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
        if tags.is_empty() {
            let msg = if self.shown.trouble.is_some() { "no picture" } else { "point it at a tag" };
            g.text_at(side.cx(), side.cy() - 8.0, Align::Center, msg, ty.body, t.dim);
            g.text_at(side.cx(), side.cy() + 10.0, Align::Center, "36h11 · 6.5 in", ty.caption, t.faint);
        }
        let card_h = ((side.h - 4.0 * 2.0) / 3.0).floor();
        for (i, tag) in tags.iter().take(3).enumerate() {
            let card = Rect::new(side.x, side.y + i as f32 * (card_h + 4.0), side.w, card_h);
            g.surface(card, g.radius(8.0).min(card_h * 0.4), t.surface1);
            // Its number and which way it is along the top; how far, and how
            // far it is turned, along the bottom.
            let way = tag.pose.map(|p| bearing(p.bearing())).unwrap_or_default();
            let name = card_name(g, tag.id, &way, card.w);
            g.text_at(card.x + 8.0, card.y + 10.0, Align::Left, &name, ty.caption, t.dim);
            match tag.pose {
                Some(pose) => {
                    let big = distance_style(g, card.w);
                    let w = g.text(card.x + 8.0, card.bottom() - 7.0, &format!("{:.2}", pose.distance()), big, t.ink);
                    g.text(card.x + 10.0 + w, card.bottom() - 7.0, "m", ty.caption, t.dim);
                    g.text_at(card.right() - 8.0, card.y + 10.0, Align::Right, &way, ty.caption, t.dim);
                    let yaw = format!("yaw {:.0}°", pose.yaw());
                    g.text_at(card.right() - 8.0, card.bottom() - 11.0, Align::Right, &yaw, ty.caption, t.dim);
                }
                None => {
                    g.text_at(card.x + 8.0, card.cy() + 6.0, Align::Left, "no distance", ty.caption, t.faint);
                }
            }
        }
        // How the camera is doing.
        let source = if self.simulated { "simulated" } else { "camera" };
        let lens = if self.calibrated { "calibrated lens" } else { "nominal lens" };
        let state = if self.paused { "held".to_owned() } else { format!("{:.0} fps", self.shown.fps) };
        g.hint(&format!("{source} · {state} · {lens}"));
    }
}

/// Where the picture goes (4:3, as tall as the page allows above the hint
/// line), and the column beside it for the tags.
fn layout(g: &Gfx) -> (Rect, Rect) {
    let ty = g.ty;
    let top = g.top();
    let ph = (H - 2.0 - ty.caption.size - 6.0 - top).floor();
    let frame = Rect::new(PAD, top, (ph * 4.0 / 3.0).floor(), ph);
    (frame, Rect::new(frame.right() + 10.0, top, W - frame.right() - 10.0 - PAD, ph))
}

/// The distance's type: the display voice at 17, a step smaller where the
/// larger text leaves no room for it beside the yaw.
fn distance_style(g: &mut Gfx, card_w: f32) -> bezel::TextStyle {
    let caption = g.ty.caption;
    let rest = 2.0 + g.measure("m", caption) + g.measure("yaw -89°", caption) + 22.0;
    for size in [17.0, 15.5, 14.0] {
        let style = g.ty.display(size);
        if g.measure("9.99", style) + rest <= card_w {
            return style;
        }
    }
    g.ty.display(14.0)
}

/// A tag's name on its card: "tag 7", or just "7" where the larger text
/// leaves no room for the word beside which way it is.
fn card_name(g: &mut Gfx, id: i32, way: &str, card_w: f32) -> String {
    let caption = g.ty.caption;
    let room = card_w - 22.0 - g.measure(way, caption);
    let full = format!("tag {id}");
    if g.measure(&full, caption) <= room {
        full
    } else {
        id.to_string()
    }
}

/// Which way a tag is: ahead, or how many degrees to the left or right.
fn bearing(degrees: f64) -> String {
    if degrees.abs() < 1.0 {
        "ahead".into()
    } else {
        format!("{:.0}° {}", degrees.abs(), if degrees > 0.0 { "right" } else { "left" })
    }
}

fn distance(tag: &Tag) -> f64 {
    tag.pose.map_or(f64::MAX, |p| p.distance())
}

/// The preview's greys into `frame`, each pixel of it as many of the panel's
/// as fit.
fn draw_preview(g: &mut Gfx, frame: Rect, preview: &[u8]) {
    let (w, h) = (g.canvas.w as usize, g.canvas.h as usize);
    let (x0, y0) = (frame.x.round() as usize, frame.y.round() as usize);
    let (fw, fh) = (frame.w as usize, frame.h as usize);
    for y in 0..fh.min(h.saturating_sub(y0)) {
        let sy = y * PREVIEW_H / fh;
        for x in 0..fw.min(w.saturating_sub(x0)) {
            let v = preview[sy * PREVIEW_W + x * PREVIEW_W / fw] as u32;
            g.canvas.px[(y0 + y) * w + x0 + x] = (v << 16) | (v << 8) | v;
        }
    }
}

/// A straight line `width` pixels wide.
fn line(g: &mut Gfx, a: (f32, f32), b: (f32, f32), width: f32, color: Color) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (nx, ny) = (-dy / len * width * 0.5, dx / len * width * 0.5);
    g.canvas.polygon(&[(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)], color, 1.0);
}

// ------------------------------------------------------ simulated camera ---

/// The simulated camera's picture at `t` seconds: a grey room with tag 7 a
/// metre or so away, drifting and turning, and tag 13 further back, drawn as
/// the nominal lens would see them, with a little sensor noise.
fn simulate(t: f32, y: &mut Vec<u8>, scratch: &mut Vec<u8>) {
    y.resize(CAM_W * CAM_H, 0);
    // The room: a wall, lighter at the top, and a floor.
    for row in 0..CAM_H {
        let base = if row < 330 { 120 + (row as i32 * -30 / 330) + 30 } else { 95 - (row as i32 - 330) / 10 };
        y[row * CAM_W..(row + 1) * CAM_W].fill(base.clamp(0, 255) as u8);
    }
    let tag = |id: i32, at: [f32; 3], yaw: f32, y: &mut Vec<u8>, scratch: &mut Vec<u8>| draw_tag(id, at, yaw, y, scratch);
    tag(13, [-0.55, -0.12, 2.6], 0.25, y, scratch);
    let (s1, s2, s3) = ((t * 0.5).sin(), (t * 0.31).sin(), (t * 0.43).sin());
    tag(7, [0.22 * s2, 0.04 * (t * 0.7).sin(), 1.25 + 0.35 * s1], 0.5 * s3, y, scratch);
    // Sensor noise, the same pattern shifted each frame.
    let shift = (t * 15.0) as u32;
    for (i, p) in y.iter_mut().enumerate() {
        // A 32-bit hash: five bits of it, centred on 0.
        let h = ((i as u32).wrapping_add(shift.wrapping_mul(7919)).wrapping_mul(2_654_435_761) >> 27) as i32 - 16;
        *p = (*p as i32 + h / 4).clamp(0, 255) as u8;
    }
}

/// Tag `id` (its full width, white border and all, 10 cells of 0.1651 / 8 m)
/// with its centre at `at` (metres from the camera: right, down, ahead),
/// turned `yaw` radians about the vertical, into the picture. Each pixel is
/// the average of four rays through it, so its edges are smooth.
fn draw_tag(id: i32, at: [f32; 3], yaw: f32, y: &mut [u8], scratch: &mut Vec<u8>) {
    let Some((cells, side)) = apriltag::tag_pixels(id) else { return };
    scratch.clone_from(&cells);
    let cell = FRC_TAG_SIZE as f32 / 8.0;
    let half = cell * side as f32 * 0.5;
    let (sin, cos) = yaw.sin_cos();
    let (eu, ev) = ([cos, 0.0, sin], [0.0, 1.0, 0.0]);
    let normal = [-sin, 0.0, cos];
    let lens = NOMINAL;
    let project = |p: [f32; 3]| (lens.fx as f32 * p[0] / p[2] + lens.cx as f32, lens.fy as f32 * p[1] / p[2] + lens.cy as f32);
    // Where its corners land, for the box to draw in.
    let corners: Vec<(f32, f32)> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|(u, v)| project([at[0] + eu[0] * u * half, at[1] + v * half, at[2] + eu[2] * u * half]))
        .collect();
    let x0 = corners.iter().map(|c| c.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
    let x1 = corners.iter().map(|c| c.0).fold(f32::MIN, f32::max).ceil().min(CAM_W as f32 - 1.0) as usize;
    let y0 = corners.iter().map(|c| c.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
    let y1 = corners.iter().map(|c| c.1).fold(f32::MIN, f32::max).ceil().min(CAM_H as f32 - 1.0) as usize;
    let along = normal[0] * at[0] + normal[2] * at[2];
    for py in y0..=y1 {
        for px in x0..=x1 {
            let (mut sum, mut hits) = (0u32, 0u32);
            for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let d = [(px as f32 + ox - lens.cx as f32) / lens.fx as f32, (py as f32 + oy - lens.cy as f32) / lens.fy as f32, 1.0];
                let facing = normal[0] * d[0] + normal[2] * d[2];
                if facing.abs() < 1e-6 {
                    continue;
                }
                let k = along / facing;
                let p = [d[0] * k - at[0], d[1] * k - at[1], d[2] * k - at[2]];
                let (u, v) = (p[0] * eu[0] + p[2] * eu[2], p[1] * ev[1]);
                let (cu, cv) = ((u + half) / cell, (v + half) / cell);
                if cu < 0.0 || cv < 0.0 || cu >= side as f32 || cv >= side as f32 {
                    continue;
                }
                sum += scratch[cv as usize * side + cu as usize] as u32;
                hits += 1;
            }
            if hits > 0 {
                // Paper under room light: white is not quite white.
                let paper = (sum / hits) as i32 * 200 / 255 + 20;
                let at = py * CAM_W + px;
                y[at] = ((y[at] as i32 * (4 - hits as i32) + paper * hits as i32) / 4).clamp(0, 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_simulated_camera_shows_tags_the_detector_reads() {
        let (mut y, mut scratch) = (Vec::new(), Vec::new());
        let mut detector = Detector::new(1, 2.0);
        for t in [0.0, 1.7, 3.9] {
            simulate(t, &mut y, &mut scratch);
            let found = detector.detect(&y, CAM_W, CAM_H, FRC_TAG_SIZE, Some(NOMINAL));
            let near = found.iter().find(|tag| tag.id == 7).expect("tag 7 read");
            // Drawn 0.9 to 1.6 m away: read back within a few centimetres.
            let (s1, s2) = ((t * 0.5).sin(), (t * 0.31).sin());
            let truth = ((0.22 * s2).powi(2) + (1.25 + 0.35 * s1).powi(2)).sqrt();
            let read = near.pose.unwrap().distance() as f32;
            assert!((read - truth).abs() < 0.06, "at {t} s: {read} m read, {truth} m drawn");
            // And how far it is turned (right edge back is positive), within a few degrees.
            let (yaw, drawn) = (near.pose.unwrap().yaw() as f32, (0.5 * (t * 0.43).sin()).to_degrees());
            assert!((yaw - drawn).abs() < 6.0, "at {t} s: turned {yaw}° read, {drawn}° drawn");
            assert!(found.iter().any(|tag| tag.id == 13), "and tag 13 behind it");
        }
    }

    #[test]
    fn a_tags_card_fits_its_words_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            let (_, side) = layout(&g);
            // The widest each line can be: tag 586 (36h11's last), 89° left; and
            // as far and as turned as a tag can be and still be read at
            // 640 × 480 (well under 10 m, under 90° from square on).
            let way = bearing(-89.0);
            let name = card_name(&mut g, 586, &way, side.w);
            if size == crate::gfx::TextSize::Standard {
                assert_eq!(name, "tag 586", "the word fits at the usual size");
            }
            let top = g.measure(&name, ty.caption) + g.measure(&way, ty.caption);
            let big = distance_style(&mut g, side.w);
            if size == crate::gfx::TextSize::Standard {
                assert_eq!(big.size, ty.display(17.0).size, "full size at the usual size");
            }
            let bottom = g.measure("9.99", big) + 2.0 + g.measure("m", ty.caption) + g.measure("yaw -89°", ty.caption);
            assert!(top + 22.0 <= side.w, "{size:?}: the top line, {top} in {}", side.w);
            assert!(bottom + 22.0 <= side.w, "{size:?}: the bottom line, {bottom} in {}", side.w);
            assert!(g.measure("simulated · 15 fps · calibrated lens", ty.caption) <= W - 2.0 * PAD, "{size:?}: the hint");
        }
    }

    #[test]
    fn a_calibration_file_is_read_and_a_bad_one_ignored() {
        crate::shell::tests::sandbox();
        let dir = crate::config_dir().join("apriltag");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lens.json"), r#"{"fx": 600.5, "fy": 601.0, "cx": 318.0, "cy": 242.0}"#).unwrap();
        assert_eq!(lens(), (Lens { fx: 600.5, fy: 601.0, cx: 318.0, cy: 242.0 }, true));
        std::fs::write(dir.join("lens.json"), r#"{"fx": -1}"#).unwrap();
        assert_eq!(lens(), (NOMINAL, false));
        std::fs::remove_file(dir.join("lens.json")).ok();
    }
}
