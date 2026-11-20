//! About, laid out as Android's "About phone": the handheld's name large at
//! the top, then its model, the Catalyst Linux version (a page of its own),
//! its status, the open-source licences, and the build number at the bottom.
//!
//! As on Android, Enter seven times on Build number makes you a developer
//! (Settings gains Developer options), and Enter three times in quick
//! succession on the version opens an Easter egg: confetti in your colours,
//! your team, and a line from the pits.

use std::time::{Duration, Instant};

use bezel::{Align, Color, Rect, TextStyle};
use detent::{role, Animated};

use crate::brand;
use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// Presses on Build number that make a developer.
const TO_DEVELOPER: u32 = 7;
/// Presses on the version that open the Easter egg, and how close together.
const TO_EGG: u32 = 3;
const EGG_WITHIN: Duration = Duration::from_millis(900);
/// The header's height at rest on the first page.
const HERO_H: f32 = 48.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Main,
    Version,
    Status,
    Legal,
}

/// What Enter does on a row.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Act {
    Nothing,
    Open(Kind),
    Build,
    Version,
}

struct Row {
    name: &'static str,
    value: String,
    act: Act,
}

pub struct About {
    kind: Kind,
    list: ListState,
    rows: Vec<Row>,
    read_at: Instant,
    /// Presses so far on Build number or the version, and when the last was.
    presses: u32,
    last_press: Option<Instant>,
    /// The header on the first page: 1 shown, 0 folded away as the list goes down.
    hero: Animated,
}

/// The pieces Catalyst Linux is made of that others wrote, with their licences.
const PIECES: [(&str, &str); 16] = [
    ("Linux kernel", "GPL-2.0"),
    ("BusyBox", "GPL-2.0"),
    ("musl libc", "MIT"),
    ("OpenRC", "BSD-2-Clause"),
    ("Mesa", "MIT"),
    ("Dropbear SSH", "MIT"),
    ("curl", "curl"),
    ("Mbed TLS", "Apache-2.0"),
    ("Git", "GPL-2.0"),
    ("wpa_supplicant", "BSD-3-Clause"),
    ("Raspberry Pi firmware", "Broadcom"),
    ("M5Stack overlays", "GPL-2.0"),
    ("Google Sans Flex", "OFL-1.1"),
    ("Google Sans Code", "OFL-1.1"),
    ("Material Symbols", "Apache-2.0"),
    ("Rust crates", "MIT · Apache-2.0"),
];

/// The build number: the version and when it was built (from
/// `/etc/catalyst/version`), as `0.2.0.20261119.1210`; on a computer, `sim`.
pub fn build_number() -> String {
    let text = std::fs::read_to_string("/etc/catalyst/version").unwrap_or_default();
    let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('='))).map(|v| v.trim().to_owned());
    let version = crate::sysupdate::version();
    match field("build").and_then(|b| b.parse::<i64>().ok()).and_then(|b| chrono::DateTime::from_timestamp(b, 0)) {
        Some(when) => format!("{version}.{}", when.format("%Y%m%d.%H%M")),
        None => format!("{version}.sim"),
    }
}

/// When this system was built, as a date, if it says.
fn built() -> Option<String> {
    let text = std::fs::read_to_string("/etc/catalyst/version").ok()?;
    let build = text.lines().find_map(|l| l.strip_prefix("build=")).and_then(|b| b.trim().parse::<i64>().ok())?;
    chrono::DateTime::from_timestamp(build, 0).map(|d| d.format("%-d %b %Y").to_string())
}

fn dash(v: Option<String>) -> String {
    v.unwrap_or_else(|| "–".into())
}

/// The handheld's name: its hostname, `catalyst` as the image sets it.
fn device_name() -> String {
    crate::sys::hostname().unwrap_or_else(|| "catalyst".into())
}

/// What it is, from the device tree; on a computer, the simulator.
fn model() -> String {
    crate::sys::model().unwrap_or_else(|| (if cfg!(target_arch = "aarch64") { "–" } else { "Simulator" }).into())
}

/// The team: from the robot's address, else the number setup saved.
fn team(robot: &Robot) -> Option<u32> {
    crate::schedule::team_of(&robot.address).or_else(|| {
        let saved = std::fs::read_to_string(crate::config_dir().join("robot")).ok()?;
        saved.trim().parse::<u32>().ok().filter(|t| (1..100_000).contains(t))
    })
}

impl About {
    pub fn new(kind: Kind) -> Self {
        let mut hero = Animated::new(1.0, role::page()).with_rest(0.004);
        hero.jump(if kind == Kind::Main { 1.0 } else { 0.0 });
        let mut about = About { kind, list: ListState::default(), rows: Vec::new(), read_at: Instant::now(), presses: 0, last_press: None, hero };
        about.read();
        about
    }

    fn read(&mut self) {
        use crate::sys;
        self.read_at = Instant::now();
        let row = |name, value: String, act| Row { name, value, act };
        let version = crate::sysupdate::version();
        self.rows = match self.kind {
            Kind::Main => vec![
                row("Device name", device_name(), Act::Nothing),
                row("Model", model(), Act::Nothing),
                row("Catalyst Linux version", format!("{version} ›"), Act::Open(Kind::Version)),
                row("Status", format!("{} ›", dash(sys::uptime().map(|u| format!("up {u}")))), Act::Open(Kind::Status)),
                row("Open-source licences", "›".into(), Act::Open(Kind::Legal)),
                row("Build number", build_number(), Act::Build),
            ],
            Kind::Version => vec![
                row("Catalyst Linux version", version, Act::Version),
                row("Update channel", crate::sysupdate::pref("channel", "stable"), Act::Nothing),
                row("Built", dash(built()), Act::Nothing),
                row("Kernel", dash(sys::kernel()), Act::Nothing),
                row("Shell", format!("catalyst-shell {}", env!("CARGO_PKG_VERSION")), Act::Nothing),
                row("Graphics", crate::gpu::note(), Act::Nothing),
            ],
            Kind::Status => vec![
                row("Up for", dash(sys::uptime()), Act::Nothing),
                row("Memory in use", dash(sys::memory()), Act::Nothing),
                row("This shell", dash(sys::shell_memory()), Act::Nothing),
                row("Storage", dash(sys::storage()), Act::Nothing),
                row("Battery", dash(sys::battery()), Act::Nothing),
                row("Processor", dash(sys::cpu_temperature()), Act::Nothing),
                row("Ethernet", dash(sys::address("eth0")), Act::Nothing),
                row("Wi-Fi", dash(sys::address("wlan0")), Act::Nothing),
            ],
            Kind::Legal => PIECES.iter().map(|(name, licence)| row(name, (*licence).to_owned(), Act::Nothing)).collect(),
        };
    }

    /// Enter on Build number: one press nearer to developer options.
    fn press_build(&mut self, cx: &mut Cx) {
        if cx.settings.developer {
            *cx.say = Some("you are already a developer".into());
            return;
        }
        self.presses += 1;
        let left = TO_DEVELOPER.saturating_sub(self.presses);
        if left == 0 {
            cx.settings.developer = true;
            crate::save_settings(cx.settings);
            *cx.say = Some("you are now a developer".into());
        } else if self.presses >= 3 {
            *cx.say = Some(format!("{left} {} to go", if left == 1 { "press" } else { "presses" }));
        }
    }

    /// Enter on the version: three in quick succession open the Easter egg.
    fn press_version(&mut self) -> Nav {
        let now = Instant::now();
        let quick = self.last_press.is_some_and(|at| now.duration_since(at) <= EGG_WITHIN);
        self.presses = if quick { self.presses + 1 } else { 1 };
        self.last_press = Some(now);
        if self.presses >= TO_EGG {
            self.presses = 0;
            return Nav::Push(Box::new(Confetti::new()));
        }
        Nav::Stay
    }

    fn draw_hero(&mut self, g: &mut Gfx, top: f32, shown: f32) {
        let t = g.tokens;
        let ty = g.ty;
        let h = HERO_H * shown;
        if shown < 0.02 {
            return;
        }
        // Folding away, it slides up under the title as it fades.
        let y = top - (1.0 - shown) * HERO_H * 0.5;
        g.canvas.set_clip(Rect::new(0.0, top, W, h));
        // Catalyst's icon: the bolt on its own dark ground.
        let side = 36.0;
        let icon = Rect::new(PAD + 2.0, y + (HERO_H - 6.0 - side) * 0.5, side, side);
        g.canvas.rrect(icon, side * 114.0 / 512.0, Color::hex(0x15161b), shown);
        g.canvas.circle(icon.cx(), icon.cy(), side * 0.36, brand::CRIMSON, 0.14 * shown);
        brand::mark(g, icon.cx(), icon.cy(), side * 0.58, brand::CRIMSON, shown);
        let name = device_name();
        let x = icon.right() + 10.0;
        let big = ty.display(20.0);
        let name = g.text.fit(&name, &big, W - x - PAD - 6.0);
        g.text_alpha(x, icon.cy() - 6.0, &name, big, t.ink, shown);
        let line = format!("catalyst linux {} · {}", crate::sysupdate::version(), crate::sysupdate::pref("channel", "stable"));
        g.text_alpha(x + 1.0, icon.cy() + 12.0, &line, ty.caption, t.dim, shown);
        g.canvas.clear_clip();
    }
}

impl Screen for About {
    fn title(&self) -> &str {
        match self.kind {
            Kind::Main => "About",
            Kind::Version => "Version",
            Kind::Status => "Status",
            Kind::Legal => "Licences",
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let act = self.rows.get(self.list.focus).map_or(Act::Nothing, |r| r.act);
        let nav = match key {
            Key::Back | Key::Left => return Nav::Pop,
            Key::Enter | Key::Right => match act {
                Act::Open(kind) => Nav::Push(Box::new(About::new(kind))),
                Act::Build if key == Key::Enter => {
                    self.press_build(cx);
                    Nav::Stay
                }
                Act::Version if key == Key::Enter => self.press_version(),
                _ => Nav::Stay,
            },
            other => {
                crate::screens::list_key(&mut self.list, other, self.rows.len());
                Nav::Stay
            }
        };
        // The header folds away once the list goes past its first rows.
        if self.kind == Kind::Main {
            let want = if self.list.focus >= 2 { 0.0 } else { 1.0 };
            if self.hero.target() != want {
                self.hero.set(want);
            }
        }
        nav
    }

    fn tick(&mut self, dt: f32) -> bool {
        let moving = self.hero.tick(dt);
        if self.read_at.elapsed().as_secs_f32() < 2.0 {
            return moving;
        }
        let before: Vec<String> = self.rows.iter().map(|r| r.value.clone()).collect();
        self.read();
        moving || before != self.rows.iter().map(|r| r.value.clone()).collect::<Vec<_>>()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if g.calm {
            self.hero.jump(self.hero.target());
        }
        let top = g.top();
        let shown = self.hero.value().clamp(0.0, 1.0);
        if self.kind == Kind::Main {
            self.draw_hero(g, top, shown);
        }
        let list_top = top + HERO_H * shown;
        let (row_h, bottom) = (g.row_h(), H - PAD + 2.0);
        for (i, y) in self.list.layout(g, self.rows.len(), list_top, row_h, 3.0, bottom) {
            let row = &self.rows[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let ink = if row.act == Act::Nothing && self.kind != Kind::Legal { t.dim } else { t.ink };
            g.row(r, row.name, &row.value, ink, i == self.list.focus);
        }
        self.list.scrollbar(g, self.rows.len(), list_top, bottom - 2.0);
    }
}

// ------------------------------------------------------------ easter egg ---

/// Lines from the pits, one at a time, in no fixed order.
const LINES: &[&str] = &[
    // The robot.
    "It worked in simulation.",
    "Have you tried power-cycling the robot?",
    "That brownout was a feature.",
    "CAN bus at 100%. So is morale.",
    "Loop overrun? Never heard of it.",
    "Battery 3 still sags to 6 volts.",
    "Ten volts is still a battery. Ish.",
    "Swerve: eight motors, one opinion.",
    "PID tuned by vibes.",
    "Odometry says we're in the lobby.",
    "Autonomous worked. Once. In the pits.",
    "Is the robot enabled? Is it, though?",
    "Elevator: going up, eventually.",
    "Pneumatics: the hiss means it works.",
    "Somewhere, a loose CAN wire.",
    "The camera saw an AprilTag. Probably.",
    "The intake ate the bumper again.",
    "Zero warnings. Suspicious.",
    "Not a bug. A match strategy.",
    // The pits.
    "Ship it. Fix it in the pits.",
    "One more deploy before queueing.",
    "Zip ties: load-bearing since forever.",
    "Duct tape is a valid fastener.",
    "Every bolt is the 10 mm one.",
    "Check the breaker before the code.",
    "Somebody unplugged the radio again.",
    "Bumpers on. Battery in. Breathe.",
    "Inspection passed. Celebrate quietly.",
    "The field is never quite level.",
    "Safety glasses on. Yes, even now.",
    "First rule of the pits: snacks.",
    // The team.
    "Gracious professionalism, now at 60 fps.",
    "Drive team: this is your sign to hydrate.",
    "The scouts saw everything.",
    "Pit crew: legends, every one of them.",
    "Mentor says it'll be fine. Probably.",
    "Comment your code. Please.",
    "Coffee is a robot subsystem.",
    // This handheld.
    "Built in Rust. Driven by Java.",
    "Springs by Detent. Glass by Bezel.",
    "Systemcore says hello.",
    "Forty frames a second, all yours.",
];

/// How long a line shows before the next.
const LINE_SECONDS: f32 = 3.6;

/// One piece of confetti.
struct Bit {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    angle: f32,
    spin: f32,
    w: f32,
    h: f32,
    /// Which of the palette's colours.
    colour: usize,
    age: f32,
    life: f32,
    /// A paper's flutter: its own phase.
    phase: f32,
    round: bool,
}

/// The Easter egg: the bolt on its glow, your team, a line from the pits,
/// and confetti in your colours. Enter throws more; Esc goes back.
pub struct Confetti {
    ground: Vec<u32>,
    bits: Vec<Bit>,
    palette: Vec<Color>,
    seed: u32,
    /// Throw a burst at the next draw, once the palette is known.
    burst_due: bool,
    line: usize,
    line_at: Instant,
    words: Animated,
    pop: Animated,
    calm: bool,
}

impl Confetti {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.subsec_nanos()) | 1;
        let mut words = Animated::new(0.0, role::smooth()).with_rest(0.004);
        words.set(1.0);
        Confetti {
            ground: brand::ground(),
            bits: Vec::new(),
            palette: Vec::new(),
            seed,
            burst_due: true,
            line: (seed as usize) % LINES.len(),
            line_at: Instant::now(),
            words,
            pop: Animated::new(1.0, role::wobble()).with_rest(0.002),
            calm: false,
        }
    }

    fn random(&mut self) -> f32 {
        // xorshift32: plenty for paper.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// A burst from the bolt: up and out, to fall and flutter. In the calm
    /// version the paper lies where it would have landed, still.
    fn burst(&mut self, count: usize) {
        let (cx, cy) = (W * 0.5, 50.0);
        for _ in 0..count {
            let angle = (-90.0 + (self.random() - 0.5) * 150.0f32).to_radians();
            let speed = 110.0 + self.random() * 190.0;
            let mut bit = Bit {
                x: cx + (self.random() - 0.5) * 16.0,
                y: cy + (self.random() - 0.5) * 12.0,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed,
                angle: self.random() * std::f32::consts::TAU,
                spin: (self.random() - 0.5) * 14.0,
                w: 2.5 + self.random() * 3.5,
                h: 1.5 + self.random() * 1.5,
                colour: (self.random() * self.palette.len().max(1) as f32) as usize,
                age: 0.0,
                life: 2.4 + self.random() * 1.4,
                phase: self.random() * std::f32::consts::TAU,
                round: self.random() < 0.22,
            };
            if self.calm {
                bit.x = 8.0 + self.random() * (W - 16.0);
                bit.y = 8.0 + self.random() * (H - 16.0);
                bit.vx = 0.0;
                bit.vy = 0.0;
                bit.spin = 0.0;
                bit.life = f32::INFINITY;
            }
            self.bits.push(bit);
        }
        if !self.calm {
            self.pop.jump(1.16);
            self.pop.set(1.0);
        }
    }

    /// Another line, any but the one showing.
    fn next_line(&mut self) {
        let pick = (self.random() * LINES.len() as f32) as usize % LINES.len();
        self.line = if pick == self.line { (pick + 1) % LINES.len() } else { pick };
        self.line_at = Instant::now();
        if !self.calm {
            self.words.jump(0.0);
            self.words.set(1.0);
        }
    }
}

impl Screen for Confetti {
    fn title(&self) -> &str {
        "Catalyst"
    }

    fn full_screen(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Leave | Key::Ctrl('q') => Nav::Pop,
            Key::Enter | Key::Char(' ') => {
                if self.calm {
                    self.bits.clear();
                }
                self.burst(if self.calm { 40 } else { 56 });
                self.next_line();
                Nav::Stay
            }
            _ => Nav::Stay,
        }
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.words.tick(dt) | self.pop.tick(dt);
        // A new line every few seconds, by the clock: so it turns over even
        // once the paper has settled and nothing is moving.
        if self.line_at.elapsed().as_secs_f32() >= LINE_SECONDS {
            self.next_line();
            moving = true;
        }
        if dt > 0.0 && !self.calm {
            const GRAVITY: f32 = 280.0;
            let drag = (-1.6 * dt).exp();
            for b in &mut self.bits {
                b.age += dt;
                b.vx *= drag;
                b.vy = b.vy * drag + GRAVITY * dt;
                // Paper flutters side to side as it falls.
                b.x += (b.vx + (b.age * 7.0 + b.phase).sin() * 22.0) * dt;
                b.y += b.vy * dt;
                b.angle += b.spin * dt;
            }
            self.bits.retain(|b| b.age < b.life && b.y < H + 10.0);
        }
        moving || (!self.calm && !self.bits.is_empty())
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        self.calm = g.calm;
        self.palette = vec![t.signal, t.ice, t.amber, brand::CRIMSON, Color::WHITE, t.signal.mix(Color::WHITE, 0.45)];
        if self.burst_due {
            self.burst_due = false;
            self.burst(if self.calm { 40 } else { 70 });
        }
        g.canvas.px.copy_from_slice(&self.ground);
        let (cx, cy) = (W * 0.5, 50.0);
        // The glow the logo sits in.
        for k in 0..8 {
            g.canvas.circle(cx, cy, 52.0 - k as f32 * 5.0, brand::CRIMSON, 0.035);
        }
        brand::mark(g, cx, cy, 58.0 * self.pop.value(), brand::CRIMSON, 1.0);
        // The team, when the robot says which; otherwise the wordmark.
        match team(robot) {
            Some(team) => {
                let (thin, bold) = (brand::thin_style(), brand::name_style());
                let number = team.to_string();
                let (tw, nw) = (g.measure("team ", thin), g.measure(&number, bold));
                let left = ((W - tw - nw) * 0.5).round();
                g.text_alpha(left, 102.0, "team ", thin, brand::INK_THIN, 1.0);
                g.text_alpha(left + tw, 102.0, &number, bold, brand::INK_STRONG, 1.0);
            }
            None => brand::wordmark(g, W * 0.5, 102.0, (1.0, 0.0), (1.0, 0.0)),
        }
        let style = TextStyle::body(g.ty.body.size);
        let line = g.text.fit(LINES[self.line], &style, W - 24.0);
        let width = g.measure(&line, style);
        let alpha = self.words.value().clamp(0.0, 1.0);
        g.text_alpha(((W - width) * 0.5).round(), 131.0, &line, style, Color::hex(0xc9ccd3), alpha);
        let hint = "enter: more confetti · esc: back";
        g.text_at(W * 0.5, H - 9.0, Align::Center, hint, g.ty.caption, brand::INK_FAINT);
        // The paper, in front of everything.
        for b in &self.bits {
            let fade = ((b.life - b.age) / 0.6).clamp(0.0, 1.0);
            let colour = self.palette.get(b.colour).copied().unwrap_or(Color::WHITE);
            if b.round {
                g.canvas.circle(b.x, b.y, b.h * 0.9, colour, fade);
            } else {
                // A strip of paper, turning: it narrows as it tumbles edge-on.
                let (s, c) = b.angle.sin_cos();
                let (hw, hh) = (b.w * 0.5, b.h * 0.5 * (0.35 + 0.65 * (b.age * 5.0 + b.phase).cos().abs()));
                let corner = |dx: f32, dy: f32| (b.x + dx * c - dy * s, b.y + dx * s + dy * c);
                let pts = [corner(-hw, -hh), corner(hw, -hh), corner(hw, hh), corner(-hw, hh)];
                g.canvas.polygon(&pts, colour, fade);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_from_the_pits_fits_the_panel() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let style = TextStyle::body(g.ty.body.size);
            for line in LINES {
                assert!(g.measure(line, style) <= W - 24.0, "{line}");
            }
        }
    }

    #[test]
    fn confetti_flies_falls_and_is_gone() {
        let mut egg = Confetti::new();
        egg.palette = vec![Color::WHITE];
        egg.burst(50);
        assert_eq!(egg.bits.len(), 50);
        let start: f32 = egg.bits.iter().map(|b| b.y).sum::<f32>() / 50.0;
        for _ in 0..12 {
            egg.tick(1.0 / 60.0);
        }
        let risen: f32 = egg.bits.iter().map(|b| b.y).sum::<f32>() / egg.bits.len() as f32;
        assert!(risen < start, "thrown up first");
        for _ in 0..60 * 5 {
            egg.tick(1.0 / 60.0);
        }
        assert!(egg.bits.is_empty(), "and gone after a few seconds");
    }
}

#[cfg(test)]
mod about_tests {
    use super::*;

    fn press(about: &mut About, settings: &mut crate::shell::Settings, key: Key) -> (Nav, Option<String>) {
        let mut robot = Robot::none();
        let mut say = None;
        let nav = about.key(key, &mut Cx { robot: &mut robot, settings, say: &mut say, at_rest: true });
        (nav, say)
    }

    #[test]
    fn seven_presses_on_build_number_make_a_developer() {
        crate::shell::tests::sandbox();
        let mut settings = crate::shell::Settings::default();
        let mut about = About::new(Kind::Main);
        about.list.focus = about.rows.iter().position(|r| r.act == Act::Build).unwrap();
        let mut said = Vec::new();
        for _ in 0..6 {
            said.push(press(&mut about, &mut settings, Key::Enter).1);
        }
        assert!(!settings.developer);
        assert_eq!(said[..2], [None, None], "the first presses say nothing, as on Android");
        assert_eq!(said[5].as_deref(), Some("1 press to go"));
        let (_, said) = press(&mut about, &mut settings, Key::Enter);
        assert!(settings.developer);
        assert_eq!(said.as_deref(), Some("you are now a developer"));
        let (_, said) = press(&mut about, &mut settings, Key::Enter);
        assert_eq!(said.as_deref(), Some("you are already a developer"));
    }

    #[test]
    fn three_quick_presses_on_the_version_open_the_easter_egg() {
        let mut settings = crate::shell::Settings::default();
        let mut about = About::new(Kind::Version);
        assert_eq!(about.rows[0].act, Act::Version);
        assert!(matches!(press(&mut about, &mut settings, Key::Enter).0, Nav::Stay));
        assert!(matches!(press(&mut about, &mut settings, Key::Enter).0, Nav::Stay));
        assert!(matches!(press(&mut about, &mut settings, Key::Enter).0, Nav::Push(_)), "the third opens it");
        // Slow presses do not add up.
        about.press_version();
        about.last_press = about.last_press.map(|at| at - Duration::from_secs(2));
        assert!(matches!(about.press_version(), Nav::Stay));
        assert_eq!(about.presses, 1, "begun again");
    }
}
