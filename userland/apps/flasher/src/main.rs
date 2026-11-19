//! Catalyst Flasher: Catalyst Linux onto an SD card in one go.
//!
//! Three steps, left to right: the system (the newest stable or nightly
//! release, or an image file dropped on the flasher's icon), the card (only
//! removable drives are offered), and Flash. It downloads the image and
//! checks it against the release's own hash, writes it with the partition
//! table last, reads every byte back, and says when the card is ready.
//!
//!     catalyst-flasher [image.img.gz]          the window
//!     catalyst-flasher --shot DIR              its screens, as PNGs

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod disks;

use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bezel::{Align, Canvas, Color, Rect, Text, TextStyle, Tokens};

const W: usize = 720;
const H: usize = 440;
const CRIMSON: Color = Color::hex(0xe94560);
const REPO: &str = "TomAs-1226/CatalystLinux";

/// What will be written.
#[derive(Clone, Debug, PartialEq)]
enum Source {
    /// The newest release on a channel, once found.
    Release { channel: String, found: Option<Found> },
    File(PathBuf),
}

/// A release's card image: where it is and what it must hash to.
#[derive(Clone, Debug, PartialEq)]
struct Found {
    version: String,
    url: String,
    name: String,
    gz_size: u64,
    gz_sha256: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Job {
    Idle,
    /// Flash was pressed once: press again to erase the card.
    Confirm(std::time::Instant),
    Downloading(f32),
    Writing(f32),
    Checking(f32),
    Done,
    Failed(String),
}

struct App {
    source: Source,
    /// Whether the nightly channel is offered next, and the file given.
    file: Option<PathBuf>,
    disks: Vec<disks::Disk>,
    disk: usize,
    focus: usize,
    job: Arc<Mutex<Job>>,
    finding: Arc<Mutex<Option<Result<Found, String>>>>,
}

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = std::process::Command::new("curl").args(["-fsSL", "--retry", "3", "-A", "catalyst-flasher"]).args(args).output().map_err(|e| format!("could not run curl: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// The newest release on a channel, and its card image.
fn find(channel: &str) -> Result<Found, String> {
    let body = curl(&[&format!("https://api.github.com/repos/{REPO}/releases?per_page=20")]).map_err(|e| format!("could not reach GitHub: {e}"))?;
    let all: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let release = all
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["draft"] == false && r["prerelease"].as_bool() == Some(channel == "nightly") && r["assets"].as_array().is_some_and(|a| a.iter().any(|f| f["name"] == "os_list_catalyst.json")))
        .ok_or(format!("no {channel} release yet"))?;
    let asset = |name: &str| release["assets"].as_array().into_iter().flatten().find(|a| a["name"].as_str().is_some_and(|n| n == name || (name == "*.img.gz" && n.ends_with(".img.gz")))).and_then(|a| a["browser_download_url"].as_str()).map(str::to_owned);
    let list: serde_json::Value = serde_json::from_slice(&curl(&[&asset("os_list_catalyst.json").ok_or("no image list")?])?).map_err(|e| e.to_string())?;
    let os = &list["os_list"][0];
    let url = asset("*.img.gz").ok_or("the release has no card image")?;
    Ok(Found {
        version: os["name"].as_str().unwrap_or_default().trim_start_matches("Catalyst Linux ").to_owned(),
        name: url.rsplit('/').next().unwrap_or("catalyst-linux.img.gz").to_owned(),
        url,
        gz_size: os["image_download_size"].as_u64().unwrap_or(0),
        gz_sha256: os["image_download_sha256"].as_str().unwrap_or_default().to_owned(),
        size: os["extract_size"].as_u64().unwrap_or(0),
        sha256: os["extract_sha256"].as_str().unwrap_or_default().to_owned(),
    })
}

fn hash_file(path: &std::path::Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(disks::sha2_hex(&h.finalize()))
}

/// The whole job, on its own thread: fetch (if need be), write, read back.
fn flash(source: Source, disk: disks::Disk, job: Arc<Mutex<Job>>) {
    let set = |j: Job| *job.lock().expect("job") = j;
    let run = || -> Result<(), String> {
        let (path, expect, length) = match &source {
            Source::Release { found: Some(f), .. } => {
                let dir = std::env::temp_dir().join("catalyst-flasher");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let to = dir.join(&f.name);
                if hash_file(&to).ok().as_deref() != Some(f.gz_sha256.as_str()) {
                    let child = std::process::Command::new("curl").args(["-fsSL", "--retry", "3", "-C", "-", "-A", "catalyst-flasher", "-o"]).arg(&to).arg(&f.url).spawn();
                    let mut child = child.map_err(|e| format!("could not run curl: {e}"))?;
                    loop {
                        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                            if !status.success() && status.code() != Some(33) {
                                return Err("the download stopped: try again".into());
                            }
                            break;
                        }
                        let got = std::fs::metadata(&to).map_or(0, |m| m.len());
                        set(Job::Downloading(got as f32 / f.gz_size.max(1) as f32));
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                    if hash_file(&to)? != f.gz_sha256 {
                        std::fs::remove_file(&to).ok();
                        return Err("the download came out wrong (its hash): try again".into());
                    }
                }
                (to, Some(f.sha256.clone()), f.size)
            }
            Source::Release { found: None, .. } => return Err("the release has not been found yet".into()),
            Source::File(p) => (p.clone(), None, 0),
        };
        set(Job::Writing(0.0));
        disks::prepare(&disk)?;
        let file = std::fs::File::open(&path).map_err(|e| format!("opening the image: {e}"))?;
        let gz = path.extension().is_some_and(|e| e == "gz");
        let total = if length > 0 { length } else if gz { 0 } else { std::fs::metadata(&path).map_or(0, |m| m.len()) };
        let mut reader: Box<dyn Read> = if gz { Box::new(flate2::read::GzDecoder::new(std::io::BufReader::new(file))) } else { Box::new(std::io::BufReader::new(file)) };
        let (len, hash) = disks::write(&disk, &mut reader, &mut |n| set(Job::Writing(if total > 0 { n as f32 / total as f32 } else { 0.5 })))?;
        if let Some(want) = &expect {
            if *want != hash {
                return Err("the image did not unpack as released".into());
            }
        }
        if len > disk.size {
            return Err("the image is larger than the card".into());
        }
        set(Job::Checking(0.0));
        let back = disks::read_back(&disk, len, &mut |n| set(Job::Checking(n as f32 / len.max(1) as f32)))?;
        if back != hash {
            return Err("the card did not read back as written: try another card".into());
        }
        disks::refresh(&disk);
        Ok(())
    };
    set(match run() {
        Ok(()) => Job::Done,
        Err(why) => Job::Failed(why),
    });
}

/// The brand's ground: a dark diagonal with a faint crimson pool.
fn ground(canvas: &mut Canvas) {
    for y in 0..H {
        for x in 0..W {
            let t = ((x as f32 / W as f32 + y as f32 / H as f32) * 0.5).clamp(0.0, 1.0);
            let c = Color::hex(0x191a20).mix(Color::hex(0x08090b), t);
            let (dx, dy) = ((x as f32 / W as f32 - 0.12) / 0.5, (y as f32 / H as f32 - 0.1) / 0.6);
            let pool = (1.0 - (dx * dx + dy * dy).sqrt()).max(0.0);
            canvas.px[y * W + x] = c.mix(CRIMSON, pool * 0.10).to_u32();
        }
    }
}

/// The bolt of FrcCatalyst's logo.
fn bolt(canvas: &mut Canvas, cx: f32, cy: f32, height: f32) {
    const BOLT: [(f32, f32); 6] = [(13.5, 2.0), (5.0, 13.0), (10.0, 13.0), (8.5, 22.0), (19.0, 10.0), (13.5, 10.0)];
    let s = height / 20.0;
    let pts: Vec<(f32, f32)> = BOLT.iter().map(|(x, y)| (cx + (x - 12.0) * s, cy + (y - 12.0) * s)).collect();
    canvas.glow(cx, cy, height * 1.4, CRIMSON, 0.22);
    canvas.polygon(&pts, CRIMSON, 1.0);
}

/// Text centred on `cy`.
#[allow(clippy::too_many_arguments)]
fn put(text: &mut Text, canvas: &mut Canvas, x: f32, cy: f32, s: &str, style: TextStyle, color: Color, align: Align) -> f32 {
    text.draw_aligned(canvas, x, (cy + style.size * 0.36).round(), align, s, &style, color, 1.0)
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

impl App {
    fn ready(&self) -> bool {
        let source = match &self.source {
            Source::Release { found, .. } => found.is_some(),
            Source::File(_) => true,
        };
        source && !self.disks.is_empty()
    }

    fn busy(&self) -> bool {
        matches!(*self.job.lock().expect("job"), Job::Downloading(_) | Job::Writing(_) | Job::Checking(_))
    }

    fn find(&mut self, channel: &str) {
        self.source = Source::Release { channel: channel.into(), found: None };
        let (slot, channel) = (self.finding.clone(), channel.to_owned());
        *slot.lock().expect("finding") = None;
        std::thread::spawn(move || {
            let found = find(&channel);
            *slot.lock().expect("finding") = Some(found);
        });
    }

    /// Enter (or a click) on a step.
    fn act(&mut self, step: usize, args_for_elevation: &mut Option<Vec<String>>) {
        if self.busy() {
            return;
        }
        match step {
            0 => match &self.source {
                Source::Release { channel, .. } if channel == "stable" => self.find("nightly"),
                Source::Release { .. } => match self.file.clone() {
                    Some(f) => self.source = Source::File(f),
                    None => self.find("stable"),
                },
                Source::File(_) => self.find("stable"),
            },
            1 => {
                self.disks = disks::list();
                if !self.disks.is_empty() {
                    self.disk = (self.disk + 1) % self.disks.len();
                }
            }
            _ => {
                let mut job = self.job.lock().expect("job");
                match *job {
                    Job::Confirm(at) if at.elapsed().as_secs() < 6 && self.ready() => {
                        let disk = self.disks[self.disk].clone();
                        if !disks::can_write() {
                            // Carry on in a copy with the rights to write a card.
                            let source = match &self.source {
                                Source::Release { channel, .. } => channel.clone(),
                                Source::File(p) => p.display().to_string(),
                            };
                            *args_for_elevation = Some(vec!["--source".into(), source, "--disk".into(), disk.path.display().to_string(), "--go".into()]);
                            return;
                        }
                        *job = Job::Writing(0.0);
                        drop(job);
                        let (source, job) = (self.source.clone(), self.job.clone());
                        std::thread::spawn(move || flash(source, disk, job));
                    }
                    _ if self.ready() => *job = Job::Confirm(std::time::Instant::now()),
                    _ => {}
                }
            }
        }
    }

    fn draw(&mut self, canvas: &mut Canvas, text: &mut Text) {
        if let Some(found) = self.finding.lock().expect("finding").take() {
            if let Source::Release { found: slot, .. } = &mut self.source {
                *slot = found.as_ref().ok().cloned();
            }
            if let Err(why) = found {
                *self.job.lock().expect("job") = Job::Failed(why);
            }
        }
        let t = Tokens::of(bezel::Accent::Rose, bezel::Tone::Dark);
        ground(canvas);

        // The name.
        bolt(canvas, 52.0, 58.0, 40.0);
        let name = TextStyle { tracking: -0.02, ..TextStyle::sans(30.0, 700.0) };
        let w = put(text, canvas, 82.0, 52.0, "Catalyst", name, Color::WHITE, Align::Left);
        put(text, canvas, 90.0 + w, 52.0, "Flasher", TextStyle::sans(30.0, 300.0), Color::hex(0x8d919a), Align::Left);
        put(text, canvas, 83.0, 80.0, "catalyst linux for the m5stack cardputerzero", TextStyle::label(13.0), t.dim, Align::Left);

        // The three steps.
        let job = self.job.lock().expect("job").clone();
        let (cw, ch, top, gap) = (206.0, 214.0, 118.0, 14.0);
        let left = (W as f32 - 3.0 * cw - 2.0 * gap) * 0.5;
        for i in 0..3 {
            let r = Rect::new(left + i as f32 * (cw + gap), top, cw, ch);
            canvas.rrect(r.offset(0.0, 3.0).inset(-1.0), 20.0, Color::BLACK, 0.3);
            canvas.rrect(r, 18.0, if i == self.focus { t.surface2 } else { t.surface1 }, 1.0);
            if i == self.focus {
                canvas.stroke_rrect(r.inset(-3.5), 21.5, 2.0, Color::WHITE, 1.0);
            }
            let step = ["1 · system", "2 · card", "3 · flash"][i];
            put(text, canvas, r.x + 18.0, r.y + 24.0, step, TextStyle::label(13.0), t.dim, Align::Left);
            let (big, small, hint): (String, String, String) = match i {
                0 => match &self.source {
                    Source::Release { channel, found: Some(f) } => ("Catalyst Linux".into(), format!("{} · {channel} · {}", f.version.split('-').next().unwrap_or(""), gb(f.size)), "enter: change".into()),
                    Source::Release { channel, found: None } => ("Catalyst Linux".into(), format!("finding {channel}…"), "enter: change".into()),
                    Source::File(p) => (p.file_name().map_or("an image".into(), |n| n.to_string_lossy().into_owned()), "a file on this computer".into(), "enter: change".into()),
                },
                1 => match self.disks.get(self.disk) {
                    Some(d) => (d.name.clone(), format!("{}{}", gb(d.size), if d.mounted.is_empty() { String::new() } else { format!(" · {}", d.mounted) }), if self.disks.len() > 1 { format!("enter: next of {}", self.disks.len()) } else { "enter: look again".into() }),
                    None => ("No card".into(), "put an SD card in".into(), "enter: look again".into()),
                },
                _ => match &job {
                    Job::Idle => ("Flash".into(), if self.ready() { "erases the card".into() } else { "choose a card first".into() }, "enter: flash".into()),
                    Job::Confirm(_) => ("Erase it?".into(), self.disks.get(self.disk).map_or(String::new(), |d| d.name.clone()), "enter again: go".into()),
                    Job::Downloading(p) => ("Downloading".into(), format!("{}%", (p * 100.0) as u32), String::new()),
                    Job::Writing(p) => ("Writing".into(), format!("{}%", (p * 100.0) as u32), "keep the card in".into()),
                    Job::Checking(p) => ("Checking".into(), format!("{}%", (p * 100.0) as u32), "reading it all back".into()),
                    Job::Done => ("Ready".into(), "into the CardputerZero".into(), "enter: another".into()),
                    Job::Failed(_) => ("Did not flash".into(), "see below".into(), "enter: try again".into()),
                },
            };
            let big_style = TextStyle::sans(if big.len() > 16 { 19.0 } else { 23.0 }, 650.0);
            let big = text.fit(&big, &big_style, cw - 36.0);
            put(text, canvas, r.x + 18.0, r.y + 84.0, &big, big_style, Color::WHITE, Align::Left);
            let small = text.fit(&small, &TextStyle::body(14.0), cw - 36.0);
            put(text, canvas, r.x + 18.0, r.y + 112.0, &small, TextStyle::body(14.0), t.dim, Align::Left);
            // Flash's own button: crimson when it will act, a bar while it works.
            if i == 2 {
                let button = Rect::new(r.x + 18.0, r.bottom() - 58.0, cw - 36.0, 30.0);
                let progress = match job {
                    Job::Downloading(p) => Some(p * 0.25),
                    Job::Writing(p) => Some(0.25 + p * 0.6),
                    Job::Checking(p) => Some(0.85 + p * 0.15),
                    _ => None,
                };
                match progress {
                    Some(p) => {
                        canvas.rrect(button, 15.0, t.surface3, 1.0);
                        canvas.rrect_clipped(Rect::new(button.x, button.y, (button.w * p.clamp(0.02, 1.0)).max(30.0), button.h), 15.0, button, 15.0, CRIMSON, 1.0);
                    }
                    None => {
                        let live = self.ready() && !matches!(job, Job::Done);
                        canvas.rrect(button, 15.0, if live { CRIMSON } else { t.surface3 }, 1.0);
                        let label = if matches!(job, Job::Confirm(_)) { "erase and flash" } else if matches!(job, Job::Done) { "done" } else { "flash" };
                        put(text, canvas, button.cx(), button.cy(), label, TextStyle::sans(15.0, 650.0), if live { Color::WHITE } else { t.dim }, Align::Center);
                    }
                }
            }
            if !hint.is_empty() {
                let hint = text.fit(&hint, &TextStyle::label(11.0), cw - 36.0);
                put(text, canvas, r.x + 18.0, r.bottom() - 14.0, &hint, TextStyle::label(11.0), t.faint, Align::Left);
            }
        }

        // The line along the bottom.
        let line = match &job {
            Job::Failed(why) => why.clone(),
            Job::Done => "the card is ready: Catalyst Linux grows its storage to fill it on first start".into(),
            Job::Writing(_) | Job::Checking(_) => "do not take the card out until this says it is ready".into(),
            _ if !disks::can_write() => "writing a card asks for Administrator when you flash".into(),
            _ => format!("images from github.com/{REPO}/releases · also in Raspberry Pi Imager"),
        };
        let color = if matches!(job, Job::Failed(_)) { CRIMSON } else { t.dim };
        let line = text.fit(&line, &TextStyle::label(12.0), W as f32 - 60.0);
        put(text, canvas, W as f32 * 0.5, H as f32 - 34.0, &line, TextStyle::label(12.0), color, Align::Center);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    // An image dropped on the program's icon arrives as its only argument.
    let file = value("--source").filter(|s| s != "stable" && s != "nightly").or_else(|| args.iter().find(|a| !a.starts_with("--") && (a.ends_with(".img") || a.ends_with(".img.gz"))).cloned()).map(PathBuf::from);
    let mut app = App {
        source: Source::Release { channel: "stable".into(), found: None },
        file: file.clone(),
        disks: disks::list(),
        disk: 0,
        focus: 0,
        job: Arc::new(Mutex::new(Job::Idle)),
        finding: Arc::new(Mutex::new(None)),
    };
    if let Some(dir) = value("--shot") {
        return shots(app, PathBuf::from(dir));
    }
    match (&file, value("--source")) {
        (Some(f), _) => app.source = Source::File(f.clone()),
        (None, Some(channel)) => app.find(&channel),
        (None, None) => app.find("stable"),
    }
    if let Some(path) = value("--disk") {
        app.disk = app.disks.iter().position(|d| d.path.display().to_string() == path).unwrap_or(0);
    }
    window(app, args.iter().any(|a| a == "--go"));
}

#[cfg(not(target_arch = "aarch64"))]
fn window(mut app: App, mut go: bool) {
    use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
    let mut win = Window::new("Catalyst Flasher", W, H, WindowOptions::default()).expect("a window");
    win.set_target_fps(30);
    let mut canvas = Canvas::new(W as i32, H as i32);
    let mut text = Text::new();
    let mut was_down = false;
    let mut last_list = std::time::Instant::now();
    while win.is_open() {
        let mut elevate = None;
        if go && app.ready() {
            // Started again with the rights to write: carry straight on.
            go = false;
            app.focus = 2;
            *app.job.lock().expect("job") = Job::Confirm(std::time::Instant::now());
            app.act(2, &mut elevate);
        }
        for key in win.get_keys_pressed(KeyRepeat::No) {
            match key {
                Key::Left => app.focus = app.focus.saturating_sub(1),
                Key::Right | Key::Tab => app.focus = (app.focus + 1).min(2),
                Key::Enter | Key::Space => app.act(app.focus, &mut elevate),
                Key::Escape if !app.busy() => return,
                _ => {}
            }
        }
        let down = win.get_mouse_down(MouseButton::Left);
        if down && !was_down {
            if let Some((x, y)) = win.get_mouse_pos(MouseMode::Discard) {
                let (cw, gap) = (206.0, 14.0);
                let left = (W as f32 - 3.0 * cw - 2.0 * gap) * 0.5;
                if (118.0..332.0).contains(&y) {
                    let i = ((x - left) / (cw + gap)).floor();
                    if (0.0..3.0).contains(&i) && (x - left) - i * (cw + gap) <= cw {
                        app.focus = i as usize;
                        app.act(app.focus, &mut elevate);
                    }
                }
            }
        }
        was_down = down;
        if let Some(args) = elevate {
            match disks::elevate(&args) {
                Ok(()) => return,
                Err(why) => *app.job.lock().expect("job") = Job::Failed(why),
            }
        }
        // Cards come and go: look again now and then while nothing is happening.
        if !app.busy() && last_list.elapsed().as_secs() >= 3 {
            last_list = std::time::Instant::now();
            let now = disks::list();
            if now != app.disks {
                app.disk = app.disk.min(now.len().saturating_sub(1));
                app.disks = now;
            }
        }
        app.draw(&mut canvas, &mut text);
        win.update_with_buffer(&canvas.px, W, H).ok();
    }
}

#[cfg(target_arch = "aarch64")]
fn window(_: App, _: bool) {}

/// Its screens as PNGs, with a pretend release and card.
fn shots(mut app: App, dir: PathBuf) {
    std::fs::create_dir_all(&dir).expect("the folder");
    let found = Found { version: "0.2.0".into(), url: String::new(), name: "catalyst-linux-0.2.0.img.gz".into(), gz_size: 45_000_000, gz_sha256: String::new(), size: 513_000_000, sha256: String::new() };
    app.source = Source::Release { channel: "stable".into(), found: Some(found) };
    app.disks = vec![disks::Disk { path: "test".into(), number: Some(2), name: "SanDisk Ultra".into(), size: 31_914_983_424, mounted: "E:".into() }];
    let mut canvas = Canvas::new(W as i32, H as i32);
    let mut text = Text::new();
    for (name, focus, job) in [
        ("1-ready", 0, Job::Idle),
        ("2-confirm", 2, Job::Confirm(std::time::Instant::now())),
        ("3-writing", 2, Job::Writing(0.42)),
        ("4-done", 2, Job::Done),
    ] {
        app.focus = focus;
        *app.job.lock().expect("job") = job;
        app.draw(&mut canvas, &mut text);
        let file = std::fs::File::create(dir.join(format!("{name}.png"))).expect("a png");
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W as u32, H as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png");
        let rgb: Vec<u8> = canvas.px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
        w.write_image_data(&rgb).expect("png");
    }
    println!("flasher screens in {}", dir.display());
}
