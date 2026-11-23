//! Settings › Camera › Smart camera: how the smart camera on the Grove port
//! stands, live, and what that means.
//!
//! **What it shows**, four times a second off the frame beat: the Grove 5 V
//! (as its switch reads back) and for how long; the bytes each way; the
//! frames, refused and good; the rate in use and the fastest tried; the last
//! error; the camera's hello (its program, MaixPy, its sensor); how long it
//! took to start; and its program's update over the Grove port, with a bar.
//! Above them, what it all means in plain words ([`findings`]): "no bytes
//! from the camera: check the cable or its power", "bytes but no frames:
//! the wrong speed or an old program", "the camera answered: program v2".
//!
//! **While it is open** it holds the camera (a lease asking for nothing), so
//! opening it is how to wake the camera and watch it start.
//!
//! **c** writes a short report to `/data/logs/smartcam-report.txt` (and puts
//! it on the clipboard), to show or send; **u** checks its program again and
//! sends this image's if it is older; **Enter** does what the first finding
//! asks where a key can (the Grove port to the camera, the install page for
//! what only USB-A can do).

use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use detent::{role, Animated};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::smartcam::{baud_words, Diag, Lease, Status, Update, RESTARTED_MS, WAKE_TIMEOUT};
use crate::unitv::Stamp;

// ---------------------------------------------------------------- findings ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Good,
    Note,
    Warn,
    Fault,
}

/// What the figures mean, in plain words: the whole sentence, and a short
/// one for a narrow line.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub long: String,
    pub short: String,
}

fn found(level: Level, long: impl Into<String>, short: impl Into<String>) -> Finding {
    Finding { level, long: long.into(), short: short.into() }
}

/// Everything the findings are made from.
#[derive(Clone, Debug)]
pub struct Facts {
    pub diag: Diag,
    pub status: Status,
    /// The Grove port is the smart camera's (Settings › Camera › Grove port).
    pub chosen: bool,
    pub switching: bool,
    /// This image's program, as a stamp.
    pub expected: Option<Stamp>,
    /// The handheld's own port (a switch to read back): the simulator has
    /// no board driver to miss.
    pub on_device: bool,
}

impl Facts {
    pub fn now() -> Facts {
        Facts {
            diag: crate::smartcam::diag(),
            status: crate::smartcam::status(),
            chosen: crate::smartcam::chosen(),
            switching: crate::smartcam::switching(),
            expected: crate::unitv::expected(),
            on_device: cfg!(target_arch = "aarch64"),
        }
    }

    fn powered_for(&self) -> Option<Duration> {
        self.diag.powered_at.map(|p| p.elapsed())
    }
}

/// What the figures mean, the most pressing first.
pub fn findings(f: &Facts) -> Vec<Finding> {
    let d = &f.diag;
    let mut out = Vec::new();
    if f.switching {
        out.push(found(Level::Note, "the Grove port is changing over…", "changing over…"));
        return out;
    }
    if !f.chosen {
        out.push(found(Level::Fault, "the Grove port is the console: the camera gets no power", "the Grove port is the console"));
        return out;
    }
    if f.status == Status::PortBusy {
        out.push(found(Level::Fault, "the port is not free: a login or the kernel's console holds it", "the port is not free yet"));
    }
    let powered = d.powered_at.is_some();
    if powered && d.five_volt == Some(false) {
        out.push(found(Level::Fault, "the Grove 5 V did not come on: catalyst-grove refused it", "the 5 V did not come on"));
    }
    if f.on_device && d.five_volt.is_none() && (powered || d.wakes > 0) {
        out.push(found(Level::Fault, "no switch for the Grove 5 V: the board's driver did not load", "no switch for the Grove 5 V"));
    }
    // Waiting on it, or given up on it: what came back says where it stops.
    let waited = f.powered_for().is_some_and(|p| p >= WAKE_TIMEOUT) || d.unanswered > 0;
    if d.hello.is_none() && (powered || d.unanswered > 0) {
        if d.bytes_in == 0 && waited {
            out.push(found(Level::Fault, "no bytes from the camera: check the cable, or give the UnitV its own USB-C power", "no bytes from it: the cable, or its power"));
        } else if d.bytes_in > 0 && d.frames == 0 && (waited || f.powered_for().is_some_and(|p| p >= Duration::from_secs(5))) {
            out.push(found(Level::Fault, "bytes but no frames: the wrong speed, or an old program (USB-A once)", "bytes, no frames: speed or program"));
        } else if let Some(p) = f.powered_for() {
            let s = p.as_secs();
            let what = if d.bytes_in == 0 { "no bytes yet" } else { "bytes, no answer yet" };
            out.push(found(Level::Note, format!("waking the camera: {what} · {s} s"), format!("waking · {s} s")));
        }
    }
    if let Some(h) = &d.hello {
        if let Some(why) = h.sensor.as_deref().filter(|s| *s != "ok") {
            out.push(found(Level::Fault, format!("its image sensor did not start ({why})"), "its image sensor did not start"));
        }
        if h.full == Some(false) {
            out.push(found(Level::Fault, "its MaixPy lacks what the program needs: USB-A, once", "its MaixPy: USB-A, once"));
        }
        if h.cwd == "/sd" {
            out.push(found(Level::Warn, "it starts from a microSD card: take the card out", "take its microSD card out"));
        }
        if h.kpu.is_empty() || h.kpu.starts_with("none") || h.kpu.starts_with("failed") {
            if h.version >= 2 {
                out.push(found(Level::Warn, "no face model on it: faces are off (USB-A installs it)", "no face model: faces off"));
            }
        }
    }
    let restarts = d.restarts + u32::from(d.restarts == 0 && d.start_ms.is_some_and(|s| s > RESTARTED_MS));
    if restarts > 0 {
        let times = if restarts == 1 { "once".to_owned() } else { format!("{restarts} times") };
        out.push(found(Level::Warn, format!("it restarted by itself {times}: short of power; give the UnitV its own USB-C power"), format!("restarted {times}: give it its own power")));
    }
    match &d.update {
        Update::Idle => {}
        Update::UsbOnce(why) => out.push(found(Level::Warn, why.clone(), "its program: USB-A, once")),
        Update::Sending { file, done } => {
            let pct = (done * 100.0) as u32;
            out.push(found(Level::Note, format!("updating its program over the Grove port: {file} · {pct}%"), format!("updating · {pct}%")));
        }
        Update::Restarting => out.push(found(Level::Note, "starting it again with its new program…", "starting it again…")),
        Update::Done(release) => out.push(found(Level::Good, format!("updated over the Grove port to Catalyst {release}'s program"), "its program updated")),
        Update::Failed(why) => out.push(found(Level::Fault, format!("its update stopped: {why} (u tries again)"), "its update stopped")),
        Update::RolledBack => out.push(found(Level::Warn, "its new program did not start: the old one is back", "its old program is back")),
    }
    if d.update != Update::RolledBack && d.hello.as_ref().is_some_and(|h| h.rolled_back) {
        out.push(found(Level::Warn, "its last update did not start: the old program is back", "its old program is back"));
    }
    if d.refused > 20 && d.refused * 10 > d.frames {
        out.push(found(Level::Warn, format!("many frames refused at {}: a long or loose cable", baud_words(d.baud)), "many frames refused"));
    }
    if d.lost > 0 {
        let times = if d.lost == 1 { "once".to_owned() } else { format!("{} times", d.lost) };
        out.push(found(Level::Note, format!("the link went quiet {times} and was found again"), format!("link lost {times}")));
    }
    if let Some(h) = &d.hello {
        let program = match &h.stamp {
            Some(s) if !s.catalyst.is_empty() => format!("program v{} (Catalyst {})", h.version, s.catalyst),
            _ => format!("program v{}", h.version),
        };
        let maixpy = if h.maixpy.is_empty() { String::new() } else { format!(" · MaixPy {}", h.maixpy) };
        out.push(found(Level::Good, format!("the camera answered: {program}{maixpy}"), format!("answered: program v{}", h.version)));
    }
    if out.is_empty() {
        out.push(match f.status {
            Status::Asleep => found(Level::Note, "asleep: it wakes while this page is open", "asleep"),
            _ => found(Level::Note, "waiting for the camera…", "waiting…"),
        });
    }
    // The most pressing first; in order of finding among the same.
    out.sort_by(|a, b| b.level.cmp(&a.level));
    out
}

// ------------------------------------------------------------------ report ---

fn ago(at: Instant) -> String {
    let s = at.elapsed().as_secs();
    match s {
        0..=59 => format!("{s} s ago"),
        60..=3599 => format!("{} min ago", s / 60),
        _ => format!("{} h ago", s / 3600),
    }
}

fn bytes(n: u64) -> String {
    match n {
        0..=999 => format!("{n} B"),
        1000..=999_999 => format!("{:.1} kB", n as f64 / 1e3),
        _ => format!("{:.2} MB", n as f64 / 1e6),
    }
}

fn five_words(d: &Diag) -> String {
    match (d.five_volt, d.powered_at) {
        (Some(true), Some(at)) => format!("on · {} s", at.elapsed().as_secs()),
        (Some(true), None) => "on".into(),
        (Some(false), Some(_)) => "off, asked on".into(),
        (Some(false), None) => "off".into(),
        (None, _) => "no switch to read".into(),
    }
}

fn update_words(u: &Update) -> String {
    match u {
        Update::Idle => "–".into(),
        Update::UsbOnce(_) => "USB-A, once".into(),
        Update::Sending { done, .. } => format!("sending · {}%", (done * 100.0) as u32),
        Update::Restarting => "starting again".into(),
        Update::Done(r) => format!("updated · {r}"),
        Update::Failed(_) => "stopped".into(),
        Update::RolledBack => "rolled back".into(),
    }
}

/// The short report `c` writes: the figures, the findings, the camera's own
/// words and the port's.
pub fn report(f: &Facts) -> String {
    let d = &f.diag;
    let mut r = String::new();
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_secs());
    r += &format!("Catalyst {} · smart camera report · unix {unix}\n", crate::sysupdate::version());
    r += &format!("grove port: {}\n", if f.chosen { "smart camera" } else { "console" });
    r += &format!("status: {:?}\n", f.status);
    r += &format!("5 V: {} · powered {} times · no answer {} times\n", five_words(d), d.wakes, d.unanswered);
    r += &format!("bytes: in {} · out {}\n", d.bytes_in, d.bytes_out);
    r += &format!("frames: {} good · {} refused · {} bytes of junk\n", d.frames, d.refused, d.junk);
    r += &format!("speed: {} in use · fastest tried {} · did not hold {}\n", baud_words(d.baud.max(crate::smartcam::START_BAUD)), if d.baud_tried > 0 { baud_words(d.baud_tried) } else { "–".into() }, d.baud_failed.map_or("–".into(), baud_words));
    r += &format!("start: {} · started again by itself {} · link lost {}\n", d.start_ms.map_or("–".into(), |s| format!("{:.1} s from power", f64::from(s) / 1000.0)), d.restarts, d.lost);
    r += &format!("last error: {}\n", d.last_error.as_ref().map_or("–".into(), |(at, w)| format!("{w} ({})", ago(*at))));
    r += &format!("update: {}{}\n", update_words(&d.update), match &d.update {
        Update::UsbOnce(w) | Update::Failed(w) => format!(" · {w}"),
        _ => String::new(),
    });
    r += &format!("this image's program: {}\n", f.expected.as_ref().map_or("–".into(), Stamp::json));
    r += &format!("hello: {}\n", crate::smartcam::about().unwrap_or_else(|| "–".into()));
    if let Some(port) = &d.port {
        r += &format!("port: {port}\n");
    }
    r += "findings:\n";
    for x in findings(f) {
        r += &format!("- {:?}: {}\n", x.level, x.long);
    }
    r
}

/// Where the report goes.
pub fn report_path() -> std::path::PathBuf {
    crate::logs::kept_dir().join("smartcam-report.txt")
}

/// The report written (and put on the clipboard): where, or why not.
pub fn write_report(f: &Facts) -> Result<std::path::PathBuf, String> {
    let text = report(f);
    let path = report_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    crate::sys::write_atomic(&path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    crate::clipboard::copy(&text, crate::clipboard::Sensitivity::Normal);
    Ok(path)
}

// -------------------------------------------------------------------- page ---

pub struct StatusPage {
    /// Held while the page is open: the camera powered and linked.
    _lease: Option<Lease>,
    facts: Facts,
    /// Pictures: the facts held as set.
    frozen: bool,
    list: ListState,
    /// The update's bar (or the wait's), gliding to where it stands.
    bar: Animated,
}

impl StatusPage {
    pub fn new() -> StatusPage {
        let lease = crate::smartcam::lease(crate::smartcam::Use::Status);
        let mut page = StatusPage { _lease: Some(lease), facts: Facts::now(), frozen: false, list: ListState::default(), bar: Animated::new(0.0, role::smooth()).with_rest(0.002) };
        page.bar.jump(page.progress().unwrap_or(0.0));
        page
    }

    /// The page on facts set by hand (the pictures).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn with(facts: Facts) -> StatusPage {
        let mut page = StatusPage { _lease: None, facts, frozen: true, list: ListState::default(), bar: Animated::new(0.0, role::smooth()).with_rest(0.002) };
        page.bar.jump(page.progress().unwrap_or(0.0));
        page
    }

    /// The bar: the update's progress, or the wait for a first answer.
    fn progress(&self) -> Option<f32> {
        let d = &self.facts.diag;
        match &d.update {
            Update::Sending { done, .. } => return Some(*done),
            Update::Restarting => return Some(1.0),
            _ => {}
        }
        match (&self.facts.status, d.powered_at) {
            (Status::Waking { .. }, Some(at)) if d.hello.is_none() || d.lost > 0 => Some((at.elapsed().as_secs_f32() / WAKE_TIMEOUT.as_secs_f32()).min(1.0)),
            _ => None,
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        let d = &self.facts.diag;
        let bad_five = d.powered_at.is_some() && d.five_volt == Some(false);
        let speed = match (d.hello.is_some(), d.baud_failed) {
            (false, _) => "–".to_owned(),
            (true, Some(fail)) if fail > d.baud => format!("{} · {} failed", baud_words(d.baud), if fail >= 1_000_000 { format!("{}M", f64::from(fail) / 1e6) } else { format!("{}k", f64::from(fail) / 1e3) }),
            (true, _) => baud_words(d.baud.max(crate::smartcam::START_BAUD)),
        };
        let camera = match &d.hello {
            Some(h) if !h.maixpy.is_empty() => format!("v{} · MaixPy {}", h.version, h.maixpy),
            Some(h) => format!("program v{}", h.version),
            None => "not heard".into(),
        };
        let started = match (d.start_ms, d.restarts) {
            (Some(ms), 0) => format!("{:.1} s", f64::from(ms) / 1000.0),
            (Some(ms), n) => format!("{:.1} s · {n}× again", f64::from(ms) / 1000.0),
            (None, 0) => "–".into(),
            (None, n) => format!("{n}× again"),
        };
        // When, with the name; what, as the value (cut to the row).
        let (error_name, error) = match &d.last_error {
            Some((at, words)) => (format!("Error · {}", ago(*at)), words.clone()),
            None => ("Last error".to_owned(), "none".to_owned()),
        };
        let mut rows = vec![
            ("5 V".into(), five_words(d), bad_five),
            ("Bytes".into(), format!("in {} · out {}", bytes(d.bytes_in), bytes(d.bytes_out)), false),
            ("Frames".into(), if d.refused > 0 { format!("{} · {} refused", d.frames, d.refused) } else { d.frames.to_string() }, d.refused > 20 && d.refused * 10 > d.frames),
            ("Speed".into(), speed, false),
            ("Camera".into(), camera, false),
            ("Started in".into(), started, d.restarts > 0),
            ("Program".into(), update_words(&d.update), matches!(d.update, Update::Failed(_))),
            (error_name, error, false),
        ];
        rows.extend(coprocessor_rows(d));
        rows
    }

    fn refresh(&mut self) -> bool {
        if self.frozen {
            return self.bar.value() != self.progress().unwrap_or(0.0);
        }
        let fresh = Facts::now();
        // The figures move while powered (the seconds, the bytes).
        let changed = format!("{:?}{:?}{:?}", fresh.diag, fresh.status, fresh.chosen) != format!("{:?}{:?}{:?}", self.facts.diag, self.facts.status, self.facts.chosen) || fresh.diag.powered_at.is_some();
        self.facts = fresh;
        if let Some(p) = self.progress() {
            self.bar.set(p);
        }
        changed
    }
}

impl Screen for StatusPage {
    fn title(&self) -> &str {
        "Smart camera"
    }

    fn keeps_awake(&self) -> bool {
        matches!(self.facts.diag.update, Update::Sending { .. } | Update::Restarting)
    }

    /// The bar glides on the beat only while it moves.
    fn tick(&mut self, dt: f32) -> bool {
        self.bar.tick(dt)
    }

    /// The figures looked at four times a second, off the beat: waiting on
    /// the camera never holds the frame beat.
    fn ambient(&self) -> Option<Duration> {
        Some(Duration::from_millis(250))
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        self.refresh()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Char('c') => {
                *cx.say = Some(match write_report(&self.facts) {
                    Ok(path) => format!("report in {} · copied", path.display()),
                    Err(why) => format!("no report · {why}"),
                });
            }
            Key::Char('u') => {
                crate::smartcam::retry_update();
                *cx.say = Some("its program is checked again".into());
            }
            Key::Char('t') if self.facts.diag.hello.as_ref().is_some_and(|h| h.can("calib")) => return Nav::Push(Box::new(CalibratePage::new())),
            Key::Char('t') => *cx.say = Some("the tag pose is calibrated on Catalyst's firmware · USB-A".into()),
            Key::Enter => {
                if !self.facts.chosen {
                    crate::smartcam::switch_grove(true);
                } else if matches!(self.facts.diag.update, Update::UsbOnce(_)) || self.facts.diag.hello.as_ref().is_some_and(|h| h.full == Some(false)) {
                    return Nav::Push(Box::new(crate::unitv::InstallScreen::new()));
                } else {
                    *cx.say = Some("c: a report to send · u: its program again".into());
                }
            }
            other => {
                let rows = self.rows().len();
                crate::screens::list_key(&mut self.list, other, rows);
            }
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 4.0;
        let cap_lh = ty.caption.size + 4.0;
        let found = findings(&self.facts);
        // What it means: the most pressing, whole over both lines where it
        // needs them; else the next under it.
        let first = &found[0];
        let mut lines: Vec<(String, bezel::Color)> = {
            let ink = if first.level == Level::Fault { g.fault() } else { t.ink };
            let words = [&first.long, &first.short].into_iter().find_map(|w| wrap(g, w, ty.caption, width, 2)).unwrap_or_else(|| vec![g.text.fit(&first.short, &ty.caption, width)]);
            words.into_iter().map(|w| (w, ink)).collect()
        };
        if let (1, Some(second)) = (lines.len(), found.get(1)) {
            let words = [&second.long, &second.short].into_iter().find_map(|w| wrap(g, w, ty.caption, width, 1)).unwrap_or_else(|| vec![g.text.fit(&second.short, &ty.caption, width)]);
            lines.push((words[0].clone(), if second.level == Level::Fault { g.fault() } else { t.dim }));
        }
        for (i, (words, color)) in lines.iter().enumerate() {
            g.text_at(PAD + 2.0, top + 1.0 + (0.5 + i as f32) * cap_lh, Align::Left, words, ty.caption, *color);
        }
        // The update's bar (or the wait's), under the words.
        let bar_y = top + 5.0 + 2.0 * cap_lh;
        if self.progress().is_some() {
            g.meter(Rect::new(PAD + 2.0, bar_y, width, 4.0), self.bar.value(), t.signal);
        }
        let rows_top = bar_y + 8.0;
        let rows = self.rows();
        for (i, y) in self.list.layout(g, rows.len(), rows_top, row_h, 3.0, bottom) {
            let (name, value, bad) = &rows[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let color = if *bad { g.fault() } else if value == "–" { t.dim } else { t.ink };
            // The value cut to the room its name leaves (as the row sets
            // them: Bezel's name in the mono label, its value in Sans).
            let (name_style, value_style, name_text) = if g.theme == crate::look::Theme::Bezel { (ty.label, ty.body, name.to_lowercase()) } else { (ty.body, ty.label, name.clone()) };
            let room = r.w - 30.0 - g.measure(&name_text, name_style);
            let value = if g.measure(value, value_style) <= room { value.clone() } else { g.text.fit(value, &value_style, room) };
            g.row(r, name, &value, color, self.list.focus == i);
        }
        self.list.scrollbar(g, rows.len(), rows_top, bottom);
        let hint: &[&str] = if !self.facts.chosen {
            &["enter: Grove port to smart camera · c: report", "enter: Grove port to smart camera", "enter: smart camera"]
        } else if matches!(self.facts.diag.update, Update::UsbOnce(_)) {
            &["enter: install over USB-A · c: copy a report", "enter: USB-A · c: report", "enter: USB-A"]
        } else {
            &["c: copy a report · u: its program again", "c: report · u: program again", "c: report"]
        };
        g.hint_any(hint);
    }
}

/// Catalyst Coprocessor's own rows (main.py version 3): its state and
/// clock, its pace, its memory, its clock against the handheld's, how long
/// its sensor took to wake, the tag pose's calibration.
pub fn coprocessor_rows(d: &Diag) -> Vec<(String, String, bool)> {
    let Some(h) = d.hello.as_ref().filter(|h| h.can("health")) else { return Vec::new() };
    let v = d.health.clone().unwrap_or_default();
    let n = |k: &str| v[k].as_f64();
    let state = v["state"].as_str().unwrap_or(if d.idle_since.is_some() { "idle" } else { "–" });
    let clock = n("cpu_hz").map(|hz| format!(" · {} MHz", (hz / 1e6).round())).unwrap_or_default();
    let pace = match (state, n("fps")) {
        ("active", Some(fps)) => format!("{fps:.1} a second · {}", v["p"].as_array().map(|p| p.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default()),
        _ => "nothing asked".into(),
    };
    let memory = match (n("heap"), n("gc_free")) {
        (Some(heap), Some(gc)) => format!("{:.1} MB free · python {} kB", heap / 1e6, (gc / 1024.0) as u32),
        _ => "–".into(),
    };
    let sync = match d.clock {
        Some((off, within)) => format!("{:+.1} s ahead · ±{:.1} ms", off as f64 / 1e6, within as f64 / 1e3),
        None => "–".into(),
    };
    let errors = v["errors"].as_u64().unwrap_or(0);
    let fw = if h.firmware_version > 0 { format!("{} {}", h.firmware, h.firmware_version) } else { "MaixPy of Sipeed's".into() };
    vec![
        ("Coprocessor".into(), format!("{state}{clock}"), false),
        ("Firmware".into(), fw, h.firmware_version == 0),
        ("Pace".into(), pace, false),
        ("Memory".into(), memory, false),
        ("Clock".into(), sync, false),
        ("Wake".into(), d.wake_ms.map_or("–".into(), |ms| format!("{ms} ms to its first frame")), false),
        ("Errors".into(), if errors == 0 { "none".into() } else { format!("{errors} · {}", v["last_error"].as_str().unwrap_or("?")) }, errors > 0),
        ("Tag pose".into(), if h.calibrated || v["calibrated"].as_u64() == Some(1) { "calibrated · t again".into() } else { "lens figures · t calibrates".into() }, false),
    ]
}

// -------------------------------------------------------------- calibrate ---

/// The tag sizes offered (mm across the black square): FRC's 6.5 in, then
/// what printed sheets often are.
pub const TAG_SIZES: [f64; 4] = [165.1, 150.0, 100.0, 200.0];
/// Where the tag is held, from the lens (mm).
pub const DISTANCES: [f64; 2] = [500.0, 1000.0];

/// Settings › Camera › Smart camera › t: the AprilTag pose calibrated with
/// a 36h11 tag of known size, held square-on at two known distances (a
/// tape measure from the lens): the camera measures the tag's side in
/// pixels and the pose's raw distance each time, then keeps the focal
/// length and the pose's scale they give in its /flash/intrinsics.json.
pub struct CalibratePage {
    _lease: Option<Lease>,
    size: usize,
    /// Each distance's measurement (fx it gave), and the saved result.
    measured: [Option<f64>; 2],
    saved: Option<(f64, f64)>,
    waiting: Option<std::sync::mpsc::Receiver<Result<Vec<u8>, String>>>,
    list: ListState,
}

impl CalibratePage {
    pub fn new() -> CalibratePage {
        CalibratePage { _lease: Some(crate::smartcam::lease(crate::smartcam::Use::Status)), size: 0, measured: [None, None], saved: None, waiting: None, list: ListState::default() }
    }

    #[cfg(not(target_arch = "aarch64"))]
    pub fn with(measured: [Option<f64>; 2], saved: Option<(f64, f64)>, focus: usize) -> CalibratePage {
        let mut page = CalibratePage { _lease: None, size: 0, measured, saved, waiting: None, list: ListState::default() };
        page.list.focus = focus;
        page
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        let at = |i: usize| match (self.measured[i], &self.waiting, self.list.focus == i + 1) {
            (_, Some(_), true) => "measuring…".to_owned(),
            (Some(fx), ..) => format!("fx {fx:.1} px"),
            (None, ..) => "enter measures".to_owned(),
        };
        vec![
            ("Tag".into(), format!("36h11 · {} mm", TAG_SIZES[self.size]), false),
            (format!("At {:.1} m", DISTANCES[0] / 1000.0), at(0), false),
            (format!("At {:.1} m", DISTANCES[1] / 1000.0), at(1), false),
            ("Lens".into(), self.saved.map_or_else(|| if self.measured.iter().all(Option::is_some) { "enter keeps it".into() } else { "measure both".into() }, |(fx, unit)| format!("fx {fx:.1} · {unit:.1} mm a unit")), self.saved.is_some()),
        ]
    }

    fn ask(&mut self, request: serde_json::Value) {
        self.waiting = Some(crate::smartcam::calibrate(&request));
    }
}

impl Screen for CalibratePage {
    fn title(&self) -> &str {
        "Tag pose"
    }

    fn keeps_awake(&self) -> bool {
        self.waiting.is_some()
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        let Some(rx) = &self.waiting else { return Nav::Stay };
        let Ok(answer) = rx.try_recv() else { return Nav::Stay };
        self.waiting = None;
        let v: serde_json::Value = match answer.map(|p| serde_json::from_slice(&p).unwrap_or_default()) {
            Ok(v) => v,
            Err(why) => {
                *cx.say = Some(why);
                return Nav::Stay;
            }
        };
        if v["ok"].as_u64() != Some(1) {
            *cx.say = Some(match v["err"].as_str() {
                Some("no tag") => "no 36h11 tag seen · hold it square-on, lit".into(),
                Some(other) => other.to_owned(),
                None => "the camera could not".into(),
            });
            return Nav::Stay;
        }
        match v["do"].as_str() {
            Some("measure") => {
                let d = v["d_mm"].as_f64().unwrap_or(0.0);
                if let Some(i) = DISTANCES.iter().position(|x| (x - d).abs() < 1.0) {
                    self.measured[i] = v["fx"].as_f64();
                    *cx.say = Some(format!("seen {} times · fx {:.1}", v["seen"].as_u64().unwrap_or(0), v["fx"].as_f64().unwrap_or(0.0)));
                }
            }
            Some("save") => {
                self.saved = Some((v["fx"].as_f64().unwrap_or(0.0), v["unit_mm"].as_f64().unwrap_or(0.0)));
                *cx.say = Some("tag pose calibrated · kept on the camera".into());
            }
            Some("reset") => {
                (self.measured, self.saved) = ([None, None], None);
                *cx.say = Some("back to the lens figures".into());
            }
            _ => {}
        }
        Nav::Stay
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if self.waiting.is_some() && !matches!(key, Key::Back | Key::Leave) {
            *cx.say = Some("measuring · hold it still".into());
            return Nav::Stay;
        }
        let tag_mm = TAG_SIZES[self.size];
        match (key, self.list.focus) {
            (Key::Back | Key::Leave, _) => return Nav::Pop,
            (Key::Left | Key::Right | Key::Enter, 0) => {
                let by = if key == Key::Left { TAG_SIZES.len() - 1 } else { 1 };
                self.size = (self.size + by) % TAG_SIZES.len();
                self.measured = [None, None];
            }
            (Key::Enter, i @ (1 | 2)) => self.ask(serde_json::json!({ "do": "measure", "d_mm": DISTANCES[i - 1], "tag_mm": tag_mm, "frames": 15 })),
            (Key::Enter, 3) if self.measured.iter().all(Option::is_some) => self.ask(serde_json::json!({ "do": "save", "tag_mm": tag_mm })),
            (Key::Enter, 3) => *cx.say = Some("measure at both distances first".into()),
            (Key::Char('d'), _) => self.ask(serde_json::json!({ "do": "reset" })),
            (other, _) => crate::screens::list_key(&mut self.list, other, 4),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 4.0;
        let cap_lh = ty.caption.size + 4.0;
        let what = match self.list.focus {
            1 | 2 => format!("Hold the tag square-on, {:.1} m from the lens, still; then enter.", DISTANCES[self.list.focus - 1] / 1000.0),
            3 => "Kept on the camera: tags' distances in millimetres from then on.".to_owned(),
            _ => "A 36h11 tag of known size, at two distances by tape measure.".to_owned(),
        };
        let lines = crate::help::wrap(g, &what, ty.caption, width);
        for (i, line) in lines.iter().take(2).enumerate() {
            g.text_at(PAD + 2.0, top + 1.0 + (0.5 + i as f32) * cap_lh, Align::Left, line, ty.caption, t.dim);
        }
        let rows_top = top + 5.0 + 2.0 * cap_lh;
        let rows = self.rows();
        for (i, y) in self.list.layout(g, rows.len(), rows_top, row_h, 3.0, bottom) {
            let (name, value, good) = &rows[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name_style, value_style, name_text) = if g.theme == crate::look::Theme::Bezel { (ty.label, ty.body, name.to_lowercase()) } else { (ty.body, ty.label, name.clone()) };
            let room = r.w - 30.0 - g.measure(&name_text, name_style);
            let value = if g.measure(value, value_style) <= room { value.clone() } else { g.text.fit(value, &value_style, room) };
            g.row(r, name, &value, if *good { t.ink } else if value.ends_with("measures") || value.starts_with("measure ") { t.dim } else { t.ink }, self.list.focus == i);
        }
        self.list.scrollbar(g, rows.len(), rows_top, bottom);
        let hint: &[&str] = match self.list.focus {
            0 => &["←→: the tag's size · d: forget it", "←→: size · d: forget", "←→ · d"],
            3 => &["enter: keep it · d: forget it", "enter: keep · d: forget", "enter · d"],
            _ => &["enter: measure · esc: back", "enter: measure", "enter"],
        };
        g.hint_any(hint);
    }
}

/// `text` in at most `max` lines of `width`, broken between words (None if
/// it takes more, or a word alone is wider).
fn wrap(g: &mut Gfx, text: &str, style: bezel::TextStyle, width: f32, max: usize) -> Option<Vec<String>> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let longer = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
        if g.measure(&longer, style) <= width {
            line = longer;
            continue;
        }
        if line.is_empty() || g.measure(word, style) > width {
            return None;
        }
        lines.push(std::mem::replace(&mut line, word.to_owned()));
    }
    if !line.is_empty() {
        lines.push(line);
    }
    (lines.len() <= max).then_some(lines)
}

// ------------------------------------------------------------------- shots ---

/// The page in each state, the update's progress on it and in the island,
/// at each text size: `catalyst-shell --smartcam-shots <dir>`.
#[cfg(not(target_arch = "aarch64"))]
pub fn shots(dir: &std::path::Path) {
    use crate::gfx::TextSize;
    use crate::shell::Shell;
    use crate::smartcam::Hello;
    std::fs::create_dir_all(dir).ok();
    crate::unitv::work_out_expected();
    let expected = crate::unitv::expected();
    let now = Instant::now();
    let ago_s = |s: u64| now.checked_sub(Duration::from_secs(s)).unwrap_or(now);
    let stamp = expected.clone();
    let hello = |version: u32| Hello {
        version,
        updates: version >= 2,
        maixpy: if version >= 2 { "0.6.3".into() } else { String::new() },
        full: (version >= 2).then_some(true),
        sensor: (version >= 2).then(|| "ok".into()),
        kpu: "face yolo2 320x240 (flash 0x300000)".into(),
        up_ms: 4200,
        listen_ms: Some(880),
        cwd: "/flash".into(),
        stamp: stamp.clone(),
        ..Hello::default()
    };
    let base = Diag { five_volt: Some(false), baud: crate::smartcam::START_BAUD, ..Diag::default() };
    let powered = |s: u64| Diag { five_volt: Some(true), powered_at: Some(ago_s(s)), wakes: 1, ..base.clone() };
    let linked = Diag { bytes_in: 48_210, bytes_out: 1_904, frames: 412, baud: 3_000_000, baud_tried: 3_000_000, hello: Some(hello(2)), hello_at: Some(ago_s(20)), start_ms: Some(2_300), ..powered(23) };
    let old = Stamp { catalyst: "0.9.3".into(), main: "0a1b2c3d4e5f".into(), ..expected.clone().unwrap_or_default() };
    let facts = |status: Status, diag: Diag| Facts { diag, status, chosen: true, switching: false, expected: expected.clone(), on_device: false };
    let waking = |s: u64| Status::Waking { since: ago_s(s) };
    let ready = Status::Ready { baud: 3_000_000, woke_ms: 2_300 };
    let states: Vec<(&str, Facts)> = vec![
        ("1-console", Facts { chosen: false, ..facts(Status::PortIsConsole, base.clone()) }),
        ("2-waking", facts(waking(4), powered(4))),
        ("3-no-bytes", facts(Status::Absent("no bytes from it".into()), Diag { unanswered: 2, wakes: 3, last_error: Some((ago_s(3), "no answer within 12 s of power on".into())), ..powered(2) })),
        ("4-bytes-no-frames", facts(Status::Absent("bytes, no frames".into()), Diag { bytes_in: 3_412, bytes_out: 640, junk: 3_412, unanswered: 1, wakes: 2, last_error: Some((ago_s(5), "no answer within 12 s of power on".into())), ..powered(14) })),
        ("5-answered", facts(ready.clone(), linked.clone())),
        ("6-short-of-power", facts(ready.clone(), Diag { restarts: 2, lost: 2, start_ms: Some(9_800), last_error: Some((ago_s(8), "the camera started again by itself".into())), ..linked.clone() })),
        ("7-updating", facts(ready.clone(), Diag { update: Update::Sending { file: "face.kmodel".into(), done: 0.46 }, hello: Some(Hello { stamp: Some(old.clone()), ..hello(2) }), ..linked.clone() })),
        ("8-restarting", facts(waking(2), Diag { update: Update::Restarting, hello: Some(Hello { stamp: Some(old.clone()), ..hello(2) }), ..linked.clone() })),
        ("9-updated", facts(ready.clone(), Diag { update: Update::Done(expected.as_ref().map_or("0.9.4".into(), |e| e.catalyst.clone())), ..linked.clone() })),
        ("10-usb-once", facts(Status::Ready { baud: 1_500_000, woke_ms: 2_100 }, Diag { update: Update::UsbOnce("its program is from before updates over the Grove port: once over USB-A".into()), hello: Some(Hello { stamp: Some(old.clone()), ..hello(1) }), baud: 1_500_000, baud_tried: 2_000_000, baud_failed: Some(2_000_000), ..linked.clone() })),
        ("11-rolled-back", facts(ready.clone(), Diag { update: Update::RolledBack, hello: Some(Hello { stamp: Some(old.clone()), rolled_back: true, ..hello(2) }), last_error: Some((ago_s(6), "smart camera update did not start · its old program is back".into())), ..linked.clone() })),
    ];
    let settle = |shell: &mut Shell| {
        for _ in 0..90 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
        shell.draw();
    };
    for (size, tag) in [(TextSize::Compact, "compact"), (TextSize::Standard, "standard"), (TextSize::Large, "large")] {
        for (name, f) in &states {
            // Each, and the answered ones scrolled to their last rows.
            let scrolled = match *name {
                "5-answered" | "6-short-of-power" => Some(7),
                "10-usb-once" => Some(4),
                _ => None,
            };
            for (suffix, focus) in [("", 0usize), ("-rows", scrolled.unwrap_or(0))].into_iter().filter(|(s, _)| s.is_empty() || scrolled.is_some()) {
                let mut shell = Shell::new(Robot::demo());
                shell.settings.text = size;
                shell.apply_settings();
                let mut page = StatusPage::with(f.clone());
                page.list.focus = focus;
                shell.show(Box::new(page));
                settle(&mut shell);
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}{suffix}-{tag}.png")), 3);
            }
        }
        // The update in the island, over Home: compact, then opened.
        let task = crate::liveactivity::task(crate::liveactivity::Kind::Download, "Smart camera", "sending face.kmodel");
        task.progress(0.46);
        let robot = Robot::none();
        let update = crate::sysupdate::Status::default();
        let list = crate::liveactivity::gather(&crate::liveactivity::Sources { robot: &robot, update: &update, unix: 0 }, None);
        for (name, open) in [("12-island-updating", false), ("13-island-updating-open", true)] {
            let mut shell = Shell::new(Robot::demo());
            shell.settings.text = size;
            shell.apply_settings();
            settle(&mut shell);
            shell.pretend_activities(Vec::new());
            shell.pretend_activities(list.clone());
            for _ in 0..10 {
                if shell.tick(1.0 / 60.0) {
                    shell.draw();
                }
            }
            if !open {
                shell.close_island_for_gallery();
                settle(&mut shell);
            } else {
                for _ in 0..40 {
                    if shell.tick(1.0 / 60.0) {
                        shell.draw();
                    }
                }
                shell.draw();
            }
            crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-{tag}.png")), 3);
        }
        drop(task);
    }
}

/// Catalyst Coprocessor's pages at each text size: the smart camera page
/// with its rows (idle, then active), the tag pose calibration as it goes,
/// Settings › Voice commands (its top, its words, a word being taught).
/// `catalyst-shell --coprocessor-shots <dir>`.
#[cfg(not(target_arch = "aarch64"))]
pub fn coprocessor_shots(dir: &std::path::Path) {
    use crate::gfx::TextSize;
    use crate::shell::Shell;
    use crate::smartcam::Hello;
    use crate::voicecmd::{Action, CommandsPage, Prefs, UserWord};
    std::fs::create_dir_all(dir).ok();
    crate::unitv::work_out_expected();
    let expected = crate::unitv::expected();
    let now = Instant::now();
    let ago_s = |s: u64| now.checked_sub(Duration::from_secs(s)).unwrap_or(now);
    let hello = Hello {
        version: 3,
        updates: true,
        maixpy: "0.6.2".into(),
        full: Some(true),
        sensor: Some("ok".into()),
        kpu: "not loaded".into(),
        up_ms: 64_000,
        listen_ms: Some(910),
        cwd: "/flash".into(),
        stamp: expected.clone(),
        firmware: "catalyst-coprocessor".into(),
        firmware_version: crate::unitv::firmware_number(),
        caps: ["idle", "pipes", "sync", "health", "audio", "calib"].map(String::from).to_vec(),
        ..Hello::default()
    };
    let linked = Diag {
        five_volt: Some(true),
        powered_at: Some(ago_s(64)),
        wakes: 1,
        bytes_in: 912_408,
        bytes_out: 61_220,
        frames: 3_921,
        baud: 3_000_000,
        baud_tried: 3_000_000,
        hello: Some(hello.clone()),
        hello_at: Some(ago_s(63)),
        start_ms: Some(2_240),
        clock: Some((-3_412_880, 210)),
        wake_ms: Some(214),
        idle_since: Some(ago_s(12)),
        health: Some(serde_json::json!({"state": "idle", "cpu_hz": 100_000_000, "fps": 0.0, "frames": 1_204, "errors": 0, "heap": 3_120_000, "gc_free": 628_000, "calibrated": 0})),
        health_at: Some(ago_s(1)),
        ..Diag::default()
    };
    let active = Diag { idle_since: None, health: Some(serde_json::json!({"state": "active", "p": ["tags"], "cpu_hz": 400_000_000, "fps": 14.8, "frames": 1_388, "errors": 0, "heap": 2_610_000, "gc_free": 590_000, "calibrated": 1})), ..linked.clone() };
    let facts = |diag: Diag| Facts { diag, status: Status::Ready { baud: 3_000_000, woke_ms: 2_240 }, chosen: true, switching: false, expected: expected.clone(), on_device: false };
    let settle = |shell: &mut Shell| {
        for _ in 0..90 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
        shell.draw();
    };
    let taught = Prefs { ready: true, user: vec![UserWord { action: Action::Open("Camera"), mean: vec![0.125; 64] }], ..Prefs::default() };
    for (size, tag) in [(TextSize::Compact, "compact"), (TextSize::Standard, "standard"), (TextSize::Large, "large")] {
        let mut pages: Vec<(String, Box<dyn Screen>)> = Vec::new();
        for (name, diag) in [("1-camera-idle", linked.clone()), ("2-camera-active", active.clone())] {
            let mut page = StatusPage::with(facts(diag));
            page.list.focus = 15;
            pages.push((name.into(), Box::new(page)));
        }
        pages.push(("3-pose-start".into(), Box::new(CalibratePage::with([None, None], None, 0))));
        pages.push(("4-pose-measured".into(), Box::new(CalibratePage::with([Some(244.1), None], None, 2))));
        pages.push(("5-pose-kept".into(), Box::new(CalibratePage::with([Some(244.1), Some(243.7)], Some((243.9, 82.6)), 3))));
        pages.push(("6-voice-top".into(), Box::new(CommandsPage::with(taught.clone(), 0))));
        pages.push(("7-voice-words".into(), Box::new(CommandsPage::with(taught.clone(), 14))));
        pages.push(("8-voice-yours".into(), Box::new(CommandsPage::with(taught.clone(), 23))));
        for (name, page) in pages {
            let mut shell = Shell::new(Robot::demo());
            shell.settings.text = size;
            shell.apply_settings();
            shell.show(page);
            settle(&mut shell);
            crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-{tag}.png")), 3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smartcam::Hello;

    fn facts(diag: Diag) -> Facts {
        let expected = Stamp { catalyst: "0.9.4".into(), main: "aaaaaaaaaaaa".into(), model: "bbbbbbbbbbbb".into(), ..Stamp::default() };
        Facts { diag, status: Status::Waking { since: Instant::now() }, chosen: true, switching: false, expected: Some(expected), on_device: true }
    }

    fn first(f: &Facts) -> Finding {
        findings(f).remove(0)
    }

    fn long_ago(s: u64) -> Option<Instant> {
        Instant::now().checked_sub(Duration::from_secs(s))
    }

    #[test]
    fn the_port_as_the_console_comes_first_and_alone() {
        let f = Facts { chosen: false, status: Status::PortIsConsole, ..facts(Diag::default()) };
        let all = findings(&f);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].level, Level::Fault);
        assert!(all[0].long.contains("console"), "{:?}", all[0]);
    }

    #[test]
    fn nothing_back_is_the_cable_or_its_power() {
        // Waiting: said as waiting, not yet a fault.
        let waking = facts(Diag { five_volt: Some(true), powered_at: long_ago(3), wakes: 1, ..Diag::default() });
        let w = first(&waking);
        assert_eq!(w.level, Level::Note, "{w:?}");
        assert!(w.long.contains("waking"));
        // Given up on, not a byte: the cable, or its own power.
        let none = facts(Diag { five_volt: Some(true), powered_at: long_ago(1), wakes: 2, unanswered: 1, ..Diag::default() });
        let n = first(&none);
        assert_eq!(n.level, Level::Fault);
        assert!(n.long.starts_with("no bytes from the camera") && n.long.contains("USB-C power"), "{n:?}");
        // The 5 V never came on: that, before anything about bytes.
        let dark = facts(Diag { five_volt: Some(false), powered_at: long_ago(20), wakes: 1, unanswered: 1, ..Diag::default() });
        assert!(first(&dark).long.contains("5 V did not come on"));
        // No switch to read on the handheld: the board's driver.
        let gone = facts(Diag { five_volt: None, powered_at: long_ago(2), wakes: 1, ..Diag::default() });
        assert!(first(&gone).long.contains("no switch for the Grove 5 V"));
    }

    #[test]
    fn bytes_without_frames_is_the_speed_or_an_old_program() {
        let f = facts(Diag { five_volt: Some(true), powered_at: long_ago(13), wakes: 1, unanswered: 1, bytes_in: 900, junk: 900, ..Diag::default() });
        let x = first(&f);
        assert_eq!(x.level, Level::Fault);
        assert!(x.long.starts_with("bytes but no frames"), "{x:?}");
    }

    #[test]
    fn an_answer_says_its_program_and_what_is_wrong_with_it() {
        let hello = Hello { version: 2, updates: true, maixpy: "0.6.3".into(), full: Some(true), sensor: Some("ok".into()), kpu: "face yolo2".into(), cwd: "/flash".into(), ..Hello::default() };
        let fine = facts(Diag { five_volt: Some(true), powered_at: long_ago(30), bytes_in: 5000, frames: 80, hello: Some(hello.clone()), start_ms: Some(2300), ..Diag::default() });
        let all = findings(&fine);
        assert_eq!(all.len(), 1, "{all:?}");
        assert_eq!(all[0].level, Level::Good);
        assert_eq!(all[0].long, "the camera answered: program v2 · MaixPy 0.6.3");
        // A start far longer than a start takes: it started more than once.
        let slow = facts(Diag { start_ms: Some(RESTARTED_MS + 2000), ..fine.diag.clone() });
        let s = first(&slow);
        assert_eq!(s.level, Level::Warn);
        assert!(s.long.contains("short of power") && s.long.contains("own USB-C power"), "{s:?}");
        // Its sensor, its MaixPy, a card in it.
        let broken = facts(Diag { hello: Some(Hello { sensor: Some("failed: Reset Failed".into()), ..hello.clone() }), ..fine.diag.clone() });
        assert!(first(&broken).long.contains("image sensor did not start"));
        let lacking = facts(Diag { hello: Some(Hello { full: Some(false), ..hello.clone() }), ..fine.diag.clone() });
        assert!(first(&lacking).long.contains("USB-A"));
        let card = facts(Diag { hello: Some(Hello { cwd: "/sd".into(), ..hello.clone() }), ..fine.diag.clone() });
        assert!(first(&card).long.contains("microSD"));
        // Many frames refused: the cable.
        let noisy = facts(Diag { refused: 60, frames: 100, baud: 3_000_000, ..fine.diag.clone() });
        assert!(first(&noisy).long.contains("many frames refused at 3 Mbaud"));
    }

    #[test]
    fn the_update_is_said_as_it_goes() {
        let base = Diag { five_volt: Some(true), powered_at: long_ago(30), bytes_in: 5000, frames: 80, ..Diag::default() };
        let sending = facts(Diag { update: Update::Sending { file: "main.py".into(), done: 0.42 }, ..base.clone() });
        assert!(findings(&sending).iter().any(|f| f.long == "updating its program over the Grove port: main.py · 42%"));
        let failed = facts(Diag { update: Update::Failed("no answer at 4096 of 9000 bytes".into()), ..base.clone() });
        assert_eq!(first(&failed).level, Level::Fault);
        let usb = facts(Diag { update: Update::UsbOnce("its program is from before updates over the Grove port: once over USB-A".into()), ..base.clone() });
        assert!(first(&usb).long.contains("USB-A"));
        let back = facts(Diag { update: Update::RolledBack, ..base });
        assert!(first(&back).long.contains("old one is back"));
    }

    #[test]
    fn the_report_holds_the_figures_and_the_findings() {
        let f = facts(Diag { five_volt: Some(true), powered_at: long_ago(1), wakes: 2, unanswered: 1, bytes_out: 120, last_error: Some((Instant::now(), "no answer within 12 s of power on".into())), ..Diag::default() });
        let r = report(&f);
        for want in ["smart camera report", "5 V: on", "bytes: in 0 · out 120", "last error: no answer within 12 s", "findings:", "- Fault: no bytes from the camera"] {
            assert!(r.contains(want), "{want:?} not in:\n{r}");
        }
    }
}
