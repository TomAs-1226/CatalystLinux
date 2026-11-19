//! First start: setting the handheld up, the way a computer's own setup
//! does it. The start animation gives way to a welcome; then, a page at a
//! time, the team, Wi-Fi, keys for online services, a page of choices, and
//! the look; then "just a moment" while it all takes effect, and Home.
//!
//! Setup owns the whole panel (no island, no status), sits on the brand's
//! ground, and every page arrives the same way: its title, its words and
//! its rows rise into place one after another on `release`, from the side
//! the page came from. Esc goes back a page, Tab skips one, and anything
//! skipped can be set later in Settings, or from a computer.

use bezel::{Accent, Color, Rect, TextStyle, Tokens, Tone};
use detent::role;

use crate::brand::{self, CRIMSON, INK_FAINT, INK_STRONG, INK_THIN};
use crate::gfx::{Gfx, W};
use crate::keys::{Keys, SERVICES};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, HomeStyle, Nav, Screen};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Page {
    Welcome,
    /// Setting up from a computer: the address to open and the code to give.
    Computer,
    Team,
    Wifi,
    Keys,
    Choices,
    Look,
    Finishing,
}

/// The steps the progress marks count.
const STEPS: [Page; 5] = [Page::Team, Page::Wifi, Page::Keys, Page::Choices, Page::Look];

const SURFACE: Color = Color::hex(0x1c1d23);
const SURFACE_FOCUSED: Color = Color::hex(0x272830);
const TRACK: Color = Color::hex(0x2b2d35);
const RING: Color = Color::hex(0xe9e9ee);

const LEFT: f32 = 16.0;
const TITLE_CY: f32 = 34.0;
const WORDS_CY: f32 = 51.0;
const ROWS_TOP: f32 = 64.0;
const BAR_CY: f32 = 157.0;

/// The line under each page's title. Each fits the panel's width at its
/// size (a test measures them): nothing on this screen is cut short.
const WORDS_WELCOME: &str = "Let's get this handheld ready for your robot.";
const WORDS_COMPUTER: &str = "On a computer on the same network, open:";
const WORDS_TEAM: &str = "The robot's address follows from it: 10.TE.AM.2";
const WORDS_WIFI: &str = "For updates and the event schedule.";
const WORDS_KEYS: &str = "For the event schedule and log analysis.";
const WORDS_TYPING: &str = "Enter keeps it; esc leaves it as it was.";
const WORDS_LOOK: &str = "Change any of these later in Settings.";
const CHOICES: [(&str, &str); 4] = [
    ("Open Robot when one is plugged in", "Its page opens when a robot links on Home."),
    ("Show how the last match went", "A match's summary opens once its log is in."),
    ("SSH access", "A computer may sign in with a key. Off is safest."),
    ("Calm version", "Less motion, and no transparency."),
];

/// How long "just a moment" stays, then "all set", before Home.
const WORKING: f32 = 1.5;
const ALL_SET: f32 = 2.6;

pub struct Setup {
    page: Page,
    focus: usize,
    /// Seconds since the page arrived, for its elements rising in.
    entered: f32,
    /// +1 when the page came from the right (forward), −1 from the left.
    from: f32,
    team: String,
    keys: Keys,
    /// A key being typed: which service, and what has been typed.
    typing: Option<(usize, String)>,
    ground: Vec<u32>,
    wifi: Option<crate::wifi::Wifi>,
    /// The Wi-Fi state's generation when last drawn.
    wifi_seen: u64,
    /// The words under a page's title, when something happened to say.
    note: Option<String>,
    /// The address and one-time code a computer uses, while it is offered.
    pub remote: Option<crate::remote::Remote>,
    /// Opened from Settings rather than on first start: only the computer
    /// page, and back to Settings when done.
    from_settings: bool,
}

impl Setup {
    pub fn new() -> Self {
        Setup {
            page: Page::Welcome,
            focus: 0,
            entered: 0.0,
            from: 1.0,
            team: String::new(),
            keys: Keys::load(),
            typing: None,
            ground: brand::ground(),
            wifi: None,
            wifi_seen: u64::MAX,
            note: None,
            remote: None,
            from_settings: false,
        }
    }

    /// Settings › Set up from a computer: the page offered for ten minutes,
    /// and whatever the computer sends put in place.
    pub fn from_computer() -> Self {
        let mut setup = Setup { from_settings: true, ..Self::new() };
        setup.go(Page::Computer, true);
        setup.remote = crate::remote::Remote::start();
        setup
    }

    /// Put in place what a computer sent.
    fn apply(&mut self, a: crate::remote::Answers, cx: &mut Cx) {
        if let Some(team) = a.team {
            self.team = team;
            self.save_team(cx);
        }
        crate::wifi::add_and_switch_on(&a.wifi);
        self.keys.merge(&a.keys);
        self.keys.save().ok();
        if let Some(key) = a.ssh_key {
            crate::add_ssh_key(&key).ok();
        }
        let s = &mut cx.settings;
        s.accent = a.accent.unwrap_or(s.accent);
        s.light = a.light.unwrap_or(s.light);
        s.text = a.text.unwrap_or(s.text);
        s.home = a.home.unwrap_or(s.home);
        s.open_robot = a.open_robot.unwrap_or(s.open_robot);
        s.match_report = a.match_report.unwrap_or(s.match_report);
        s.calm = a.calm.unwrap_or(s.calm);
        if let Some(on) = a.ssh {
            if on != crate::ssh_enabled() {
                crate::set_ssh(on).ok();
            }
        }
        crate::save_settings(cx.settings);
    }

    fn go(&mut self, page: Page, forward: bool) {
        self.page = page;
        self.focus = 0;
        self.entered = 0.0;
        self.from = if forward { 1.0 } else { -1.0 };
        self.typing = None;
        self.note = None;
        self.wifi = (page == Page::Wifi).then(crate::wifi::Wifi::open);
        if page != Page::Computer {
            self.remote = None;
        }
    }

    fn next(&mut self, cx: &mut Cx) {
        let page = match self.page {
            Page::Welcome | Page::Computer => Page::Team,
            Page::Team => {
                self.save_team(cx);
                Page::Wifi
            }
            Page::Wifi => Page::Keys,
            Page::Keys => {
                self.keys.save().ok();
                Page::Choices
            }
            Page::Choices => Page::Look,
            Page::Look | Page::Finishing => {
                // Everything chosen is kept, and setup is not shown again.
                crate::save_settings(cx.settings);
                crate::mark_setup_done();
                Page::Finishing
            }
        };
        self.go(page, true);
    }

    fn back(&mut self) {
        if self.from_settings {
            return;
        }
        let page = match self.page {
            Page::Welcome | Page::Computer | Page::Team => Page::Welcome,
            Page::Wifi => Page::Team,
            Page::Keys => Page::Wifi,
            Page::Choices => Page::Keys,
            Page::Look => Page::Choices,
            Page::Finishing => return,
        };
        if page != self.page {
            self.go(page, false);
        }
    }

    fn save_team(&mut self, cx: &mut Cx) {
        let team = self.team.trim().to_owned();
        if team.is_empty() {
            return;
        }
        crate::save_robot_address(&team);
        *cx.robot = Robot::connect(&crate::screens::resolve_address(&team));
    }

    /// How many rows the page has, not counting its Next button.
    fn rows(&self) -> usize {
        match self.page {
            Page::Welcome => 3,
            Page::Computer => 0,
            Page::Team => 1,
            Page::Wifi => 2,
            Page::Keys => SERVICES.len(),
            Page::Choices => 4,
            Page::Look => 4,
            Page::Finishing => 0,
        }
    }

    /// Whether the page has a Next button below its rows.
    fn has_next(&self) -> bool {
        matches!(self.page, Page::Team | Page::Wifi | Page::Keys | Page::Choices | Page::Look)
    }

    /// The `k`th element of the page rising into place: its opacity and how
    /// far it still has to go, across and down.
    fn reveal(&self, k: usize) -> (f32, f32, f32) {
        let start = k as f32 * 0.05;
        if self.entered <= start {
            return (0.0, 14.0 * self.from, 6.0);
        }
        let left = role::release().solve(1.0, 0.0, self.entered - start).0;
        (((1.0 - left) * 1.5).clamp(0.0, 1.0), left * 14.0 * self.from, left * 6.0)
    }

    fn toggle_choice(&mut self, row: usize, cx: &mut Cx) {
        let s = &mut cx.settings;
        match row {
            0 => s.open_robot = !s.open_robot,
            1 => s.match_report = !s.match_report,
            2 => {
                let on = !crate::ssh_enabled();
                if let Err(why) = crate::set_ssh(on) {
                    self.note = Some(why);
                }
            }
            _ => s.calm = !s.calm,
        }
    }

    fn change_look(&mut self, row: usize, step: i32, cx: &mut Cx) {
        let s = &mut cx.settings;
        match row {
            0 => {
                let all = Accent::ALL;
                let at = all.iter().position(|a| *a == s.accent).unwrap_or(0) as i32;
                s.accent = all[(at + step).rem_euclid(all.len() as i32) as usize];
            }
            1 => s.light = !s.light,
            2 => s.text = if step > 0 { s.text.next() } else { s.text.next().next() },
            _ => s.home = s.home.next(),
        }
    }

    // ---- drawing ----

    fn header(&self, g: &mut Gfx) {
        brand::mark(g, LEFT + 4.0, 13.0, 12.0, CRIMSON, 1.0);
        // The steps: those done, the one under way in crimson, those to come.
        let at = STEPS.iter().position(|p| *p == self.page);
        if at.is_none() && self.page != Page::Finishing {
            return;
        }
        let (w, gap) = (14.0, 4.0);
        let x0 = W - LEFT - STEPS.len() as f32 * (w + gap) + gap;
        for i in 0..STEPS.len() {
            let color = match at {
                Some(at) if i == at => CRIMSON,
                Some(at) if i < at => INK_THIN,
                None => INK_THIN,
                _ => TRACK,
            };
            g.canvas.rrect(Rect::new(x0 + i as f32 * (w + gap), 12.0, w, 3.0), 1.5, color, 1.0);
        }
    }

    fn heading(&self, g: &mut Gfx, title: &str, words: &str) {
        let (a, dx, dy) = self.reveal(0);
        g.text_alpha(LEFT + dx, TITLE_CY + dy, title, TextStyle::sans(18.0, 600.0), INK_STRONG, a);
        let (a, dx, dy) = self.reveal(1);
        let words = self.note.as_deref().unwrap_or(words);
        let fitted = g.text.fit(words, &TextStyle::sans(11.0, 400.0), W - 2.0 * LEFT);
        g.text_alpha(LEFT + dx, WORDS_CY + dy, &fitted, TextStyle::sans(11.0, 400.0), if self.note.is_some() { RING } else { INK_THIN }, a);
    }

    /// A row: a label at the left and a value at the right, `h` tall.
    fn row(&self, g: &mut Gfx, i: usize, y: f32, w: f32, h: f32, label: &str, value: &str, on: bool) -> Rect {
        let (a, dx, dy) = self.reveal(2 + i);
        let r = Rect::new(LEFT + dx, y + dy, w, h);
        if a <= 0.0 {
            return r;
        }
        let focused = self.focus == i;
        g.canvas.rrect(r, h * 0.5, if focused { SURFACE_FOCUSED } else { SURFACE }, a);
        let label_style = TextStyle::sans(11.5, if focused { 600.0 } else { 500.0 });
        g.text_alpha(r.x + 11.0, r.cy(), label, label_style, INK_STRONG, a);
        if !value.is_empty() {
            let style = TextStyle::code(9.5);
            let vw = g.measure(value, style);
            g.text_alpha(r.right() - 11.0 - vw, r.cy(), value, style, if on { INK_STRONG } else { INK_THIN }, a);
        }
        if focused {
            g.canvas.stroke_rrect(r.inset(-2.0), h * 0.5 + 2.0, 1.5, RING, 0.9 * a);
        }
        r
    }

    /// The bar along the bottom: what the keys do, and the Next button.
    fn bar(&self, g: &mut Gfx, hint: &str) {
        let (a, ..) = self.reveal(7);
        g.text_alpha(LEFT, BAR_CY, hint, TextStyle::code(9.0), INK_FAINT, a);
        if !self.has_next() {
            return;
        }
        let label = if self.page == Page::Look { "Finish" } else { "Next" };
        let style = TextStyle::sans(11.0, 600.0);
        let bw = g.measure(label, style) + 28.0;
        let r = Rect::new(W - LEFT - bw, BAR_CY - 9.0, bw, 18.0);
        // The primary action: the one crimson thing on the page besides progress.
        g.canvas.rrect(r, 9.0, CRIMSON, a);
        g.text_alpha(r.x + 14.0, r.cy(), label, style, Color::WHITE, a);
        if self.focus == self.rows() {
            g.canvas.stroke_rrect(r.inset(-2.0), 11.0, 1.5, RING, 0.9 * a);
        }
    }

    fn draw_look_preview(&self, g: &mut Gfx, settings: (Accent, bool, HomeStyle)) {
        let (accent, light, home) = settings;
        let (a, dx, dy) = self.reveal(4);
        let t = Tokens::of(accent, if light { Tone::Light } else { Tone::Dark });
        let frame = Rect::new(186.0 + dx, ROWS_TOP + dy, 118.0, 74.0);
        g.canvas.rrect(frame, 9.0, t.ground, a);
        g.canvas.stroke_rrect(frame, 9.0, 1.0, TRACK, a);
        // The island, and Home as this style lays it out.
        g.canvas.rrect(Rect::new(frame.cx() - 16.0, frame.y + 4.0, 32.0, 7.0), 3.5, t.surface2, a);
        match home {
            HomeStyle::Grid => {
                let (tw, th) = (32.0, 24.0);
                for i in 0..6 {
                    let r = Rect::new(frame.x + 8.0 + (i % 3) as f32 * (tw + 4.0), frame.y + 16.0 + (i / 3) as f32 * (th + 4.0), tw, th);
                    g.canvas.rrect(r, 5.0, t.surface1, a);
                    g.canvas.circle(r.x + 7.0, r.y + 7.0, 2.5, if i == 0 { t.ink } else { t.dim }, a);
                    if i == 0 {
                        g.canvas.stroke_rrect(r.inset(-1.5), 6.5, 1.2, t.signal, a);
                    }
                }
            }
            HomeStyle::Carousel => {
                for (i, (w, h)) in [(24.0, 38.0), (44.0, 50.0), (24.0, 38.0)].into_iter().enumerate() {
                    let cx = frame.cx() + (i as f32 - 1.0) * 37.0;
                    let r = Rect::new(cx - w * 0.5, frame.y + 41.0 - h * 0.5, w, h);
                    g.canvas.rrect(r, 6.0, if i == 1 { t.surface2 } else { t.surface1 }, a);
                    if i == 1 {
                        g.canvas.circle(r.cx(), r.y + 17.0, 4.0, t.ink, a);
                        g.canvas.stroke_rrect(r.inset(-1.5), 7.5, 1.2, t.signal, a);
                    }
                }
            }
        }
    }
}

impl Default for Setup {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for Setup {
    fn title(&self) -> &str {
        "Setup"
    }

    fn full_screen(&self) -> bool {
        true
    }

    fn wants_text(&self) -> bool {
        (self.page == Page::Team && self.focus == 0) || self.typing.is_some()
    }

    fn appear(&mut self) {
        self.entered = 0.0;
    }

    fn resume(&mut self) {
        // Back from choosing a network: say how it went.
        self.entered = 0.0;
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        // A key being typed takes the keys until Enter or Esc.
        if let Some((service, text)) = &mut self.typing {
            match key {
                Key::Char(c) if !c.is_control() && text.len() < 200 => text.push(c),
                Key::Backspace => {
                    text.pop();
                }
                Key::Enter => {
                    let (service, text) = (*service, text.clone());
                    self.keys.set(SERVICES[service].0, &text);
                    self.typing = None;
                }
                Key::Back => self.typing = None,
                _ => {}
            }
            return Nav::Stay;
        }
        let (rows, last) = (self.rows(), self.rows() + self.has_next() as usize);
        match key {
            // From Settings, Esc goes back there, and the page stops being offered.
            Key::Back if self.from_settings && self.page == Page::Computer => return Nav::Pop,
            Key::Back if self.page != Page::Welcome => self.back(),
            Key::Tab if self.has_next() => self.next(cx),
            Key::Up => self.focus = self.focus.saturating_sub(1),
            Key::Down if self.focus + 1 < last => self.focus += 1,
            // The Next button.
            Key::Enter if self.has_next() && self.focus == rows => self.next(cx),
            Key::Char(c) if self.page == Page::Team && self.focus == 0 && c.is_ascii_digit() && self.team.len() < 5 => self.team.push(c),
            Key::Backspace if self.page == Page::Team => {
                self.team.pop();
            }
            Key::Enter if self.page == Page::Team => self.next(cx),
            Key::Enter => match (self.page, self.focus) {
                (Page::Welcome, 0) => self.next(cx),
                (Page::Welcome, 1) => {
                    self.go(Page::Computer, true);
                    self.remote = crate::remote::Remote::start();
                }
                (Page::Welcome, _) => {
                    crate::mark_setup_done();
                    return Nav::Done;
                }
                (Page::Wifi, 0) => {
                    if !crate::wifi::switched_on() {
                        crate::wifi::switch(true);
                    }
                    return Nav::Push(Box::new(crate::screens::WifiScreen::new()));
                }
                (Page::Wifi, _) => self.next(cx),
                (Page::Keys, row) => self.typing = Some((row, String::new())),
                (Page::Choices, row) => self.toggle_choice(row, cx),
                (Page::Look, row) => self.change_look(row, 1, cx),
                _ => {}
            },
            Key::Left | Key::Right if self.page == Page::Look && self.focus < rows => {
                self.change_look(self.focus, if key == Key::Left { -1 } else { 1 }, cx);
            }
            Key::Left | Key::Right if self.page == Page::Choices && self.focus < rows => self.toggle_choice(self.focus, cx),
            _ => {}
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        let Some(answers) = self.remote.as_mut().and_then(|r| r.take()) else { return Nav::Stay };
        self.apply(answers, cx);
        if !self.from_settings {
            crate::mark_setup_done();
        }
        self.go(Page::Finishing, true);
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.entered += dt;
        // A page's elements have all arrived within about half a second, and
        // the last page runs on its own; otherwise it is drawn again only
        // when what it shows has changed: the computer page's status, the
        // Wi-Fi page's connection.
        let mut changed = self.entered < 0.8 || self.page == Page::Finishing;
        if let Some(remote) = &mut self.remote {
            changed |= remote.tick();
        }
        if let Some(wifi) = &self.wifi {
            let seen = wifi.generation();
            changed |= std::mem::replace(&mut self.wifi_seen, seen) != seen;
        }
        changed
    }

    fn busy(&self) -> bool {
        self.page == Page::Finishing
    }

    fn finished(&self) -> bool {
        self.page == Page::Finishing && self.entered >= ALL_SET
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        g.canvas.px.copy_from_slice(&self.ground);
        self.header(g);
        let row_w = W - 2.0 * LEFT;
        match self.page {
            Page::Welcome => {
                let (a, dx, dy) = self.reveal(0);
                g.text_alpha(LEFT + dx, TITLE_CY + 2.0 + dy, "Hi.", TextStyle::display(26.0), INK_STRONG, a);
                let (a, dx, dy) = self.reveal(1);
                g.text_alpha(LEFT + dx, WORDS_CY + 4.0 + dy, WORDS_WELCOME, TextStyle::sans(11.0, 400.0), INK_THIN, a);
                let rows = [("Set up here", "on the keys"), ("Set up from a computer", "faster typing"), ("Set up later", "go to Home")];
                for (i, (label, value)) in rows.iter().enumerate() {
                    self.row(g, i, ROWS_TOP + 6.0 + i as f32 * 24.0, row_w, 21.0, label, value, false);
                }
                self.bar(g, "enter chooses");
            }
            Page::Computer => {
                self.heading(g, "Set up from a computer", WORDS_COMPUTER);
                let (a, dx, dy) = self.reveal(2);
                match &self.remote {
                    Some(remote) => {
                        let url = remote.url();
                        g.text_alpha(LEFT + dx, 78.0 + dy, &url, TextStyle::code(11.0), INK_STRONG, a);
                        g.text_alpha(LEFT + dx, 98.0 + dy, "and give this code:", TextStyle::sans(11.0, 400.0), INK_THIN, a);
                        let code: String = remote.code().chars().flat_map(|c| [c, ' ']).collect();
                        g.text_alpha(LEFT + dx, 120.0 + dy, code.trim_end(), TextStyle::display(22.0), INK_STRONG, a);
                        remote.draw_qr(g, Rect::new(W - LEFT - 76.0 + dx, 64.0 + dy, 76.0, 76.0), a);
                        g.text_alpha(LEFT, BAR_CY, &remote.status(), TextStyle::code(9.0), INK_FAINT, a);
                    }
                    None => {
                        g.text_alpha(LEFT + dx, 84.0 + dy, "No network yet: plug in the Ethernet cable,", TextStyle::sans(11.0, 400.0), INK_STRONG, a);
                        g.text_alpha(LEFT + dx, 100.0 + dy, "or go back and set up here.", TextStyle::sans(11.0, 400.0), INK_STRONG, a);
                        self.bar(g, "esc back");
                    }
                }
            }
            Page::Team => {
                self.heading(g, "Which team is it for?", WORDS_TEAM);
                let (a, dx, dy) = self.reveal(2);
                let field = Rect::new(LEFT + dx, ROWS_TOP + 6.0 + dy, 132.0, 32.0);
                g.canvas.rrect(field, 10.0, SURFACE_FOCUSED, a);
                let shown = if self.team.is_empty() { "team" } else { &self.team };
                let style = TextStyle::display(22.0);
                let tw = if self.team.is_empty() { 0.0 } else { g.measure(&self.team, style) };
                g.text_alpha(field.x + 12.0, field.cy(), shown, style, if self.team.is_empty() { INK_FAINT } else { INK_STRONG }, a);
                if self.focus == 0 {
                    g.canvas.rrect(Rect::new(field.x + 13.0 + tw, field.cy() - 9.0, 1.5, 18.0), 0.75, CRIMSON, a);
                    g.canvas.stroke_rrect(field.inset(-2.0), 12.0, 1.5, RING, 0.9 * a);
                }
                if let Ok(team) = self.team.parse::<u32>() {
                    let (a, dx, dy) = self.reveal(3);
                    g.text_alpha(field.right() + 14.0 + dx, field.cy() + dy, &format!("robot at {}", crate::screens::resolve_address(&team.to_string())), TextStyle::code(9.5), INK_THIN, a);
                }
                self.bar(g, "esc back · tab skip");
            }
            Page::Wifi => {
                let status = self.wifi.as_ref().map(|w| w.state().status).unwrap_or_default();
                let words = if crate::wifi::switched_on() { format!("Wi-Fi: {status}") } else { WORDS_WIFI.into() };
                self.heading(g, "Connect to Wi-Fi?", &words);
                self.row(g, 0, ROWS_TOP + 6.0, row_w, 21.0, "Choose a network", "›", false);
                self.row(g, 1, ROWS_TOP + 30.0, row_w, 21.0, "Not now", "", false);
                self.bar(g, "esc back · tab skip");
            }
            Page::Keys => {
                let words = if self.typing.is_some() { WORDS_TYPING } else { WORDS_KEYS };
                self.heading(g, "Keys for online services", words);
                for (i, (name, label)) in SERVICES.iter().enumerate() {
                    let value = match &self.typing {
                        Some((row, text)) if *row == i => format!("{}▏", text.chars().rev().take(18).collect::<Vec<_>>().into_iter().rev().collect::<String>()),
                        _ => self.keys.shown(name),
                    };
                    self.row(g, i, ROWS_TOP + 2.0 + i as f32 * 22.0, row_w, 19.0, label, &value, self.keys.get(name).is_some());
                }
                self.bar(g, "esc back · tab skip");
            }
            Page::Choices => {
                let on = [g.open_robot, g.match_report, crate::ssh_enabled(), g.calm];
                // The line under the title explains whichever choice is focused.
                let words = CHOICES.get(self.focus).map_or(WORDS_LOOK, |c| c.1);
                self.heading(g, "Choose your settings", words);
                for (i, ((label, _), on)) in CHOICES.iter().zip(on).enumerate() {
                    self.row(g, i, ROWS_TOP + i as f32 * 20.0, row_w, 17.0, label, if on { "on" } else { "off" }, on);
                }
                self.bar(g, "enter switches · esc back");
            }
            Page::Look => {
                self.heading(g, "Make it yours", WORDS_LOOK);
                let accent = g.accent;
                let light = g.tokens.tone == Tone::Light;
                let rows = [
                    ("Accent", accent.name().to_owned()),
                    ("Mode", (if light { "light" } else { "dark" }).to_owned()),
                    ("Text", g.text_size.name().to_owned()),
                    ("Home", g.home.name().to_owned()),
                ];
                for (i, (label, value)) in rows.iter().enumerate() {
                    let r = self.row(g, i, ROWS_TOP + i as f32 * 20.0, 160.0, 17.0, label, "", false);
                    let (a, ..) = self.reveal(2 + i);
                    if i == 0 {
                        // The accents themselves, the chosen one ringed.
                        for (n, choice) in Accent::ALL.iter().enumerate() {
                            let x = r.right() - 12.0 - (Accent::ALL.len() - 1 - n) as f32 * 13.0;
                            if *choice == accent {
                                g.canvas.circle(x, r.cy(), 6.0, RING, a);
                                g.canvas.circle(x, r.cy(), 4.8, if self.focus == 0 { SURFACE_FOCUSED } else { SURFACE }, a);
                            }
                            g.canvas.circle(x, r.cy(), 3.6, Tokens::of(*choice, Tone::Dark).signal, a);
                        }
                    } else {
                        let style = TextStyle::code(9.5);
                        let vw = g.measure(&format!("‹ {value} ›"), style);
                        g.text_alpha(r.right() - 10.0 - vw, r.cy(), &format!("‹ {value} ›"), style, INK_STRONG, a);
                    }
                }
                self.draw_look_preview(g, (accent, light, g.home));
                self.bar(g, "← → change · esc back");
            }
            Page::Finishing => {
                let done = self.entered >= WORKING;
                let next = if self.from_settings { "Back to Settings." } else { "Home is next." };
                let (title, words) = if done { ("You're all set.", next) } else { ("Just a moment…", "Putting your choices in place.") };
                // The mark in the middle, and under it a turning arc while it works.
                brand::mark(g, W * 0.5, 58.0, 30.0, CRIMSON, 1.0);
                g.canvas.glow(W * 0.5, 58.0, 44.0, CRIMSON, 0.22);
                let style = TextStyle::sans(17.0, 600.0);
                let tw = g.measure(title, style);
                g.text_alpha((W - tw) * 0.5, 100.0, title, style, INK_STRONG, 1.0);
                let small = TextStyle::sans(11.0, 400.0);
                let ww = g.measure(words, small);
                g.text_alpha((W - ww) * 0.5, 118.0, words, small, INK_THIN, 1.0);
                if !done {
                    let spin = self.entered * 5.2;
                    g.canvas.arc(W * 0.5, 145.0, 7.0, 2.0, spin, 1.4 + 0.8 * (self.entered * 3.1).sin(), CRIMSON, 1.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_of_setup_fits_the_panel_whole() {
        let mut g = Gfx::new(Tokens::dark());
        let words = TextStyle::sans(11.0, 400.0);
        let lines = [WORDS_WELCOME, WORDS_COMPUTER, WORDS_TEAM, WORDS_WIFI, WORDS_KEYS, WORDS_TYPING, WORDS_LOOK];
        for line in lines.iter().chain(CHOICES.iter().map(|c| &c.1)) {
            let w = g.measure(line, words);
            assert!(w <= W - 2.0 * LEFT, "{line:?} is {w:.0} px, wider than {}", W - 2.0 * LEFT);
        }
        // The choices' names fit their rows beside "off".
        let label = TextStyle::sans(11.5, 600.0);
        for (name, _) in CHOICES {
            let w = g.measure(name, label) + 22.0 + g.measure("off", TextStyle::code(9.5)) + 11.0;
            assert!(w <= W - 2.0 * LEFT, "{name:?} crowds its row: {w:.0} px");
        }
    }
}
