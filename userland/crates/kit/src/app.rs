//! An app's pages and the loop that runs them: keys in, springs on, frames
//! out, only when something changed. Pages stack: one pushed slides in over
//! the one before, which drifts a third as far, as the shell's do, and its
//! rows rise into place one after another. The island at the top carries
//! the app's messages; the time and battery sit at the right, as on every
//! screen of the shell.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bezel::{Align, GlassShape, Rect};
use detent::{role, Animated};

use crate::keys::Key;
use crate::look::{IslandRest, Look, Theme};
use crate::ui::{spatial, Ui, H, PAD, W};

/// One page of an app. An app is its first page; others are pushed on top
/// of it and popped off again.
pub trait Page {
    /// The title at the top left (lowercase in Bezel's theme, as the shell's).
    fn title(&self) -> &str;
    /// A key, already read as this page wants it: outside text entry F X Z
    /// C are the arrows and Backspace is Back.
    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow;
    fn draw(&mut self, ui: &mut Ui);
    /// Advance in time. True while the page moves on its own and should be
    /// drawn again (a timer running, a simulation).
    fn tick(&mut self, _dt: f32) -> bool {
        false
    }
    /// True while letters should reach the page as letters (a text field).
    fn wants_text(&self) -> bool {
        false
    }
    /// The page draws the whole panel: no title, island or status over it.
    fn full_screen(&self) -> bool {
        false
    }
    /// The page is on top again after the one above it closed.
    fn resume(&mut self, _cx: &mut Cx) {}
}

impl Page for Box<dyn Page> {
    fn title(&self) -> &str {
        self.as_ref().title()
    }
    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        self.as_mut().key(key, cx)
    }
    fn draw(&mut self, ui: &mut Ui) {
        self.as_mut().draw(ui)
    }
    fn tick(&mut self, dt: f32) -> bool {
        self.as_mut().tick(dt)
    }
    fn wants_text(&self) -> bool {
        self.as_ref().wants_text()
    }
    fn full_screen(&self) -> bool {
        self.as_ref().full_screen()
    }
    fn resume(&mut self, cx: &mut Cx) {
        self.as_mut().resume(cx)
    }
}

/// What a key leads to.
pub enum Flow {
    Stay,
    /// A page slides in on top.
    Push(Box<dyn Page>),
    /// This page goes; the first page going ends the app.
    Pop,
    /// The app ends, and the shell comes back.
    Quit,
}

/// What a page can reach while it handles a key.
pub struct Cx<'a> {
    pub look: &'a Look,
    said: &'a mut Option<String>,
    data: &'a Path,
}

impl Cx<'_> {
    /// A few words in the island, for a moment.
    pub fn say(&mut self, words: impl Into<String>) {
        *self.said = Some(words.into());
    }

    /// The app's own folder on `/data`, kept across updates of the app (and
    /// made on first use). See [`crate::store`].
    pub fn data_dir(&self) -> &Path {
        self.data
    }

    pub fn calm(&self) -> bool {
        self.look.calm
    }
}

/// How long a message stays in the island: the shell's.
const MESSAGE_SECONDS: f32 = 2.4;
/// The frame beat while something moves: the shell's 40 a second.
const FRAME: Duration = Duration::from_millis(25);

struct Slide {
    progress: Animated,
    under: Vec<u32>,
    over: Vec<u32>,
    /// The page on its way out, kept until it has gone.
    leaving: Option<Box<dyn Page>>,
}

/// The pages of one app, with everything around them.
pub struct Runner {
    pub ui: Ui,
    name: String,
    pages: Vec<Box<dyn Page>>,
    slide: Option<Slide>,
    background: Vec<u32>,
    /// The page as last drawn, without the island and status over it.
    last_page: Vec<u32>,
    scratch: Vec<u32>,
    island_w: Animated,
    island_from: f32,
    island_words: String,
    message: Option<(String, f32)>,
    status: Status,
    status_age: f32,
    data: PathBuf,
    dirty: bool,
    moving: bool,
    quit: bool,
}

/// What the status at the top right shows.
#[derive(Default, PartialEq)]
struct Status {
    clock: String,
    battery: Option<u32>,
    wifi: bool,
    wired: bool,
}

impl Status {
    fn read(clock24: bool) -> Status {
        let now = chrono::Local::now();
        let clock = if clock24 { now.format("%H:%M").to_string() } else { now.format("%-I:%M").to_string() };
        let up = |dev: &str| std::fs::read_to_string(format!("/sys/class/net/{dev}/operstate")).is_ok_and(|s| s.trim() == "up");
        Status { clock, battery: crate::sys::battery_percent(), wifi: up("wlan0"), wired: up("eth0") }
    }
}

impl Runner {
    pub fn new(first: Box<dyn Page>, look: Look) -> Runner {
        let ui = Ui::new(look);
        let background = crate::look::wallpaper(look.wallpaper, &ui.tokens);
        let name = std::env::var("CATALYST_APP_NAME").unwrap_or_else(|_| first.title().to_owned());
        let mut runner = Runner {
            ui,
            name,
            pages: vec![first],
            slide: None,
            background,
            last_page: Vec::new(),
            scratch: Vec::new(),
            island_w: Animated::new(0.0, role::release()).with_rest(0.3),
            island_from: 0.0,
            island_words: String::new(),
            message: None,
            status: Status::read(look.clock24),
            status_age: 0.0,
            data: crate::store::data_dir(),
            dirty: true,
            moving: false,
            quit: false,
        };
        // The app arrives as a page of the shell does: its rows rise in.
        runner.ui.reveal_start();
        runner
    }

    /// Whether the app has ended.
    pub fn done(&self) -> bool {
        self.quit || self.pages.is_empty()
    }

    /// Words in the island, for a moment.
    pub fn say(&mut self, words: impl Into<String>) {
        self.message = Some((words.into(), MESSAGE_SECONDS));
        self.dirty = true;
    }

    /// The title of the page in front.
    pub fn top_title(&self) -> &str {
        self.pages.last().map_or("", |p| p.title())
    }

    pub fn key(&mut self, key: Key) {
        if key.leaves() {
            self.quit = true;
            return;
        }
        if matches!(key, Key::BrightnessDown | Key::BrightnessUp) {
            let step: i32 = if key == Key::BrightnessUp { 10 } else { -10 };
            let said = match crate::sys::brightness() {
                Some(now) if crate::sys::set_brightness((now as i32 + step).clamp(10, 100) as u32) => format!("brightness {}%", crate::sys::brightness().unwrap_or(now)),
                _ => "the backlight is on the handheld".to_owned(),
            };
            return self.say(said);
        }
        // A key lands on a page that is drawn live: a slide in progress finishes.
        if let Some(slide) = self.slide.take() {
            drop(slide.leaving);
        }
        let Some(page) = self.pages.last_mut() else { return };
        let key = if page.wants_text() { key } else { key.plain() };
        let look = self.ui.look;
        let mut said = None;
        let flow = page.key(key, &mut Cx { look: &look, said: &mut said, data: &self.data });
        if key == Key::Enter {
            self.ui.pressed();
        }
        if let Some(words) = said {
            self.say(words);
        }
        self.dirty = true;
        match flow {
            Flow::Stay => {}
            Flow::Quit => self.quit = true,
            Flow::Push(page) => self.push(page),
            Flow::Pop => self.pop(),
        }
    }

    fn push(&mut self, page: Box<dyn Page>) {
        let under = self.last_page.clone();
        self.pages.push(page);
        self.ui.focus_reset();
        self.ui.reveal_start();
        if self.ui.look.calm || under.len() != self.background.len() {
            return;
        }
        let over = self.picture();
        let mut progress = Animated::new(0.0, spatial::slow()).with_rest(0.002);
        progress.set(1.0);
        self.slide = Some(Slide { progress, under, over, leaving: None });
    }

    fn pop(&mut self) {
        let leaving = self.pages.pop();
        if self.pages.is_empty() {
            self.quit = true;
            return;
        }
        self.ui.focus_reset();
        let look = self.ui.look;
        let mut said = None;
        if let Some(page) = self.pages.last_mut() {
            page.resume(&mut Cx { look: &look, said: &mut said, data: &self.data });
        }
        if let Some(words) = said {
            self.say(words);
        }
        if look.calm || self.last_page.len() != self.background.len() {
            return;
        }
        let over = self.last_page.clone();
        let under = self.picture();
        let mut progress = Animated::new(1.0, spatial::slow()).with_rest(0.002);
        progress.set(0.0);
        self.slide = Some(Slide { progress, under, over, leaving });
    }

    /// The page in front drawn on its own, as a picture to move.
    fn picture(&mut self) -> Vec<u32> {
        self.ui.focus_static = true;
        self.draw_page();
        self.ui.focus_static = false;
        self.ui.canvas.px.clone()
    }

    /// Advance time by `dt`. True if the panel needs drawing again.
    pub fn tick(&mut self, dt: f32) -> bool {
        let mut changed = std::mem::take(&mut self.dirty);
        let was_moving = self.moving;
        // Time only passes for motion already under way.
        let dt = if was_moving { dt } else { dt.min(1.0 / 60.0) };
        let mut moving = self.ui.tick(dt) | self.island_w.tick(dt);
        if let Some(slide) = &mut self.slide {
            moving |= slide.progress.tick(dt);
            if slide.progress.is_settled() {
                self.slide = None;
            }
            changed = true;
        }
        if let Some(page) = self.pages.last_mut() {
            moving |= page.tick(dt);
        }
        if let Some((_, left)) = &mut self.message {
            *left -= dt;
            if *left <= 0.0 {
                self.message = None;
                changed = true;
            }
        }
        self.status_age += dt;
        if self.status_age >= 1.0 {
            self.status_age = 0.0;
            let now = Status::read(self.ui.look.clock24);
            if now != self.status {
                self.status = now;
                changed = true;
            }
        }
        self.moving = moving;
        changed || moving
    }

    /// Whether frames should keep coming at the beat.
    pub fn animating(&self) -> bool {
        self.moving || self.slide.is_some() || self.message.is_some()
    }

    fn draw_page(&mut self) {
        let ui = &mut self.ui;
        ui.clear(&self.background);
        ui.ox = 0.0;
        let Some(page) = self.pages.last_mut() else { return };
        ui.title_room = (W - self.island_w.value().max(ui.island_h())) * 0.5 - PAD - 8.0;
        if !page.full_screen() {
            let (t, ty) = (ui.tokens, ui.ty);
            let title = match ui.look.theme {
                Theme::Bezel => page.title().to_lowercase(),
                Theme::Catalyst => page.title().to_owned(),
            };
            let bar_cy = 4.0 + ui.island_h() * 0.5;
            let room = ui.title_room;
            ui.text_fit(PAD + 2.0, bar_cy, &title, ty.title, t.ink, room);
        }
        page.draw(ui);
        if ui.revealing() && !page.full_screen() {
            rise_in(ui, &self.background, &mut self.scratch);
        }
    }

    pub fn draw(&mut self) {
        match &self.slide {
            Some(slide) if slide.leaving.is_none() && self.ui.revealing() => {
                // Still rising as it slides in: drawn again, then moved.
                let over = self.picture();
                if let Some(slide) = &mut self.slide {
                    slide.over = over;
                }
            }
            _ => {}
        }
        if let Some(slide) = &self.slide {
            let p = slide.progress.value();
            let px = &mut self.ui.canvas.px;
            px.copy_from_slice(&self.background);
            let (under_x, over_x) = ((-p * W * 0.3).round() as i32, ((1.0 - p) * W).round() as i32);
            blit_shifted(px, &slide.under, under_x);
            blit_shifted(px, &slide.over, over_x);
        } else {
            self.draw_page();
            self.last_page.clone_from(&self.ui.canvas.px);
        }
        let full = self.pages.last().is_some_and(|p| p.full_screen()) && self.slide.is_none();
        if !full {
            self.draw_chrome();
        }
        if self.ui.take_motion_started() {
            self.moving = true;
        }
    }

    /// The status at the top right and the glass island in the middle.
    fn draw_chrome(&mut self) {
        let ui = &mut self.ui;
        let (t, ty) = (ui.tokens, ui.ty);
        let look = ui.look;
        let bar_cy = 4.0 + ui.island_h() * 0.5;
        let mut x = W - PAD - 2.0;
        if look.island != IslandRest::Clock {
            x -= ui.text_at(x, bar_cy, Align::Right, &self.status.clock, ty.caption, t.dim) + 7.0;
        }
        if let (Some(percent), false) = (self.status.battery, look.island == IslandRest::Battery) {
            let ink = if percent <= 15 { ui.fault() } else { t.dim };
            x -= ui.text_at(x, bar_cy, Align::Right, &format!("{percent}%"), ty.caption, ink) + 7.0;
        }
        for (up, icon) in [(self.status.wifi, "wifi"), (self.status.wired, "lan")] {
            if up {
                ui.icon(icon, false, x - 6.0, bar_cy, 13.0, t.dim);
                x -= 17.0;
            }
        }
        // The island: the app's words when it has some, otherwise what the
        // shell's island shows at rest (the app's name standing in for the
        // robot, which an app does not watch).
        let (label, message) = match (&self.message, look.island) {
            (Some((text, _)), _) => (text.clone(), true),
            (None, IslandRest::Clock) => (self.status.clock.clone(), false),
            (None, IslandRest::Battery) => (self.status.battery.map_or_else(|| "battery –".to_owned(), |p| format!("battery {p}%")), false),
            (None, IslandRest::Robot) => (self.name.to_lowercase(), false),
        };
        let caption = ty.caption;
        let island_h = ui.island_h();
        let room = if message { 250.0 } else { 96.0 };
        let label = ui.text.fit(&label, &caption, room);
        let want = ui.measure(&label, caption) + 34.0;
        if label != self.island_words {
            self.island_words = label.clone();
            self.island_from = self.island_w.value();
        }
        if (want - self.island_w.target()).abs() > 0.5 {
            let growing = want > self.island_w.target();
            if self.island_w.value() < 1.0 || look.calm {
                self.island_w.jump(want);
            } else {
                self.island_w.set_with(want, if growing { role::release() } else { role::smooth() });
                self.moving = true;
            }
        }
        let iw = self.island_w.value().max(island_h);
        let island = Rect::new((W - iw) * 0.5, 4.0, iw, island_h);
        ui.glass(&[GlassShape::new(island, island_h * 0.5, 0)]);
        // New words fade in once the island is past halfway to its width.
        let travel = want - self.island_from;
        let grown = if travel.abs() < 1.0 { 1.0 } else { ((self.island_w.value() - self.island_from) / travel).clamp(0.0, 1.0) };
        let words = ((grown - 0.55) / 0.45).clamp(0.0, 1.0);
        ui.canvas.set_clip(island.inset(2.0));
        ui.canvas.circle(island.x + 12.0, island.cy(), 3.0, if message { t.signal } else { t.faint }, 1.0);
        ui.text_alpha(island.x + 22.0, island.cy(), &label, caption, t.ink, words);
        ui.canvas.clear_clip();
    }
}

/// Copy a full-panel picture in, moved sideways by `dx` pixels.
fn blit_shifted(dst: &mut [u32], src: &[u32], dx: i32) {
    let (w, h) = (W as usize, H as usize);
    let shift = dx.unsigned_abs() as usize;
    if shift >= w || src.len() != w * h {
        return;
    }
    let (to, from) = if dx >= 0 { (shift, 0) } else { (0, shift) };
    let n = w - shift;
    for y in 0..h {
        dst[y * w + to..y * w + to + n].copy_from_slice(&src[y * w + from..y * w + from + n]);
    }
}

/// Pages rise in, as the shell's do: the arriving page is cut into blocks
/// at the rows where only the wallpaper shows, and each block rises into
/// place a little after the one above it. The wallpaper stays still.
fn rise_in(ui: &mut Ui, background: &[u32], scratch: &mut Vec<u32>) {
    let (w, h) = (W as usize, H as usize);
    if background.len() != w * h {
        return;
    }
    let mut px = std::mem::take(&mut ui.canvas.px);
    scratch.clear();
    scratch.extend_from_slice(&px);
    px.copy_from_slice(background);
    let tested = w - 8;
    let empty = |y: usize| scratch[y * w..y * w + tested] == background[y * w..y * w + tested];
    let (mut y, mut block) = (0, 0.0f32);
    while y < h {
        if empty(y) {
            y += 1;
            continue;
        }
        let start = y;
        while y < h && !empty(y) {
            y += 1;
        }
        if let Some(dy) = ui.rise(block.min(crate::ui::RISE_LAST)) {
            let dy = dy.round() as isize;
            for row in start..y {
                let to = row as isize + dy;
                if !(0..h as isize).contains(&to) {
                    continue;
                }
                let (from, to) = (row * w, to as usize * w);
                for x in 0..w {
                    let p = scratch[from + x];
                    if p != background[from + x] {
                        px[to + x] = p;
                    }
                }
            }
        }
        block += 1.0;
    }
    ui.canvas.px = px;
}

/// Run an app until it ends: on the handheld's panel when the shell has
/// handed it over, in a window on a computer.
pub fn run(first: impl Page + 'static) {
    let look = Look::from_env();
    let mut panel = crate::panel::open();
    let mut runner = Runner::new(Box::new(first), look);
    let mut last = Instant::now();
    loop {
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.25);
        last = now;
        if runner.tick(dt) {
            runner.draw();
            panel.present(&runner.ui.canvas);
        } else if panel.present_always() {
            panel.present(&runner.ui.canvas);
        }
        if runner.done() {
            break;
        }
        let wait = if runner.animating() { FRAME.saturating_sub(now.elapsed()) } else { Duration::from_millis(500) };
        let Some(keys) = panel.poll(wait) else { break };
        for key in keys {
            runner.key(key);
        }
        if runner.done() {
            break;
        }
    }
    if std::env::var_os("CATALYST_KIT_STATS").is_some() {
        if let Some(line) = crate::sys::memory_line() {
            eprintln!("{line}");
        }
    }
}

/// A scene of an app's gallery: its name, and the keys pressed to reach it
/// from the scene before.
pub type Scene<'a> = (&'a str, &'a [Key]);

/// Draw an app's screens to PNGs, one per scene, as the shell's gallery
/// does: each key followed by a second and a half of frames, so springs
/// have settled. Fails on an icon that is not built in.
#[cfg(not(target_arch = "aarch64"))]
pub fn gallery(first: Box<dyn Page>, look: Look, scenes: &[Scene], dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut runner = Runner::new(first, look);
    let mut saved = Vec::new();
    let settle = |runner: &mut Runner, frames: usize| {
        for _ in 0..frames {
            if runner.tick(1.0 / 60.0) {
                runner.draw();
            }
        }
    };
    settle(&mut runner, 90);
    for (name, keys) in scenes {
        for key in keys.iter() {
            runner.key(*key);
            settle(&mut runner, 90);
        }
        runner.draw();
        let path = dir.join(format!("{name}.png"));
        crate::panel::sim::save_png(&runner.ui.canvas, &path, 3)?;
        saved.push(path);
    }
    if let Some(name) = runner.ui.missing_icons().iter().next() {
        return Err(std::io::Error::other(format!("no icon called {name:?}")));
    }
    Ok(saved)
}

/// An app's `main`: run it, or with `--shots DIR` draw its gallery
/// (`--light`, `--text large`, `--theme bezel`, `--calm` and `--accent NAME`
/// change the look the gallery is drawn in). Games that deal at random get
/// the same deal in every gallery: it sets `CATALYST_SEED`.
pub fn launch<P: Page + 'static>(make: impl Fn() -> P, scenes: &[Scene]) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--shots") {
        std::env::set_var("CATALYST_SEED", "5805");
        let flag = |name: &str| args.iter().any(|a| a == name);
        let mut look = Look::from_env();
        look.light |= flag("--light");
        look.calm |= flag("--calm");
        if let Some(size) = value("--text").and_then(|s| crate::TextSize::from_name(&s)) {
            look.text = size;
        }
        if let Some(theme) = value("--theme").and_then(|s| Theme::from_name(&s)) {
            look.theme = theme;
            look.wallpaper = if theme == Theme::Bezel { crate::look::Wallpaper::Plain } else { crate::look::Wallpaper::Catalyst };
            look.accent = if theme == Theme::Bezel { bezel::Accent::Orange } else { bezel::Accent::Rose };
        }
        if let Some(accent) = value("--accent").and_then(|s| bezel::Accent::from_name(&s)) {
            look.accent = accent;
        }
        match gallery(Box::new(make()), look, scenes, Path::new(&dir)) {
            Ok(saved) => println!("{} screens drawn to {dir}", saved.len()),
            Err(why) => {
                eprintln!("the gallery failed: {why}");
                std::process::exit(1);
            }
        }
        return;
    }
    let _ = value;
    run(make());
}
