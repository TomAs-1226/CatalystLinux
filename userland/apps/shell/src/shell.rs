//! The shell: a stack of screens, the transitions between them, the glass
//! island that carries link status and messages, and the status at the top.
//!
//! Motion follows Detent: every transition is a spring that starts from what
//! is on screen, input stays live throughout, and a transition can be turned
//! round or re-entered at any point without a jump. Enter and exit share a
//! path: an app grows out of the tile it was opened from and shrinks back
//! into it; a page inside an app slides in from the right and back out.

use bezel::canvas::{blend, sd_rrect};
use bezel::{Accent, Align, Canvas, Color, GlassShape, Rect, Tokens, Tone};
use detent::{role, Animated, Spring};

use crate::control::{Control, Outcome, SHEET, SHEET_RADIUS};
use crate::gfx::{Gfx, TextSize, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::screens::Home;

pub struct Settings {
    pub light: bool,
    pub calm: bool,
    pub text: TextSize,
    pub accent: Accent,
    pub home: HomeStyle,
    /// Open the Robot app when a robot links while Home is showing.
    pub open_robot: bool,
    /// Show a match's summary once its log is pulled after the robot links.
    pub match_report: bool,
    /// The glance page's cards, in order, two to a page.
    pub glance: Vec<Widget>,
    /// The robot value pinned to the glance page (a key under `/Catalyst/`).
    pub pinned: Option<String>,
    pub theme: crate::look::Theme,
    pub wallpaper: crate::look::Wallpaper,
    pub corners: crate::look::Corners,
    pub letters: crate::look::Letters,
    pub width: crate::look::Width,
    /// Tiles across Home's grid: 3, or 2 for larger ones.
    pub columns: usize,
    /// Home's apps in the order chosen (the rest follow), those hidden, and
    /// the favourites pinned ahead of everything else.
    pub order: Vec<String>,
    pub hidden: Vec<String>,
    pub favourites: Vec<String>,
    /// The clock: 24-hour or not, with seconds or not.
    pub clock24: bool,
    pub seconds: bool,
    /// What the island shows when it has nothing to say.
    pub island: IslandRest,
    /// A greeting in the island as Home appears.
    pub greeting: bool,
    /// The robot's name, remembered from the last time it linked, for the
    /// start screen.
    pub robot_name: Option<String>,
    /// Draw the glass and growing windows on the GPU when there is one
    /// (otherwise everything is the processor's).
    pub gpu: bool,
    /// Developer options, unlocked by pressing About › Build number seven
    /// times; while they are on, how much slower motion runs (Android's
    /// animator duration scale: 1 as designed, 5 five times slower) and
    /// whether each key is shown at the bottom of the screen.
    pub developer: bool,
    pub motion_scale: f32,
    pub show_keys: bool,
    /// The tour has been taken (or skipped): it is not offered again.
    pub toured: bool,
}

/// The animation scales developer options offer.
pub const MOTION_SCALES: [f32; 6] = [0.5, 1.0, 1.5, 2.0, 5.0, 10.0];

/// How Home is arranged: apps in the order chosen, those hidden, and the
/// favourites pinned first.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Arrangement {
    pub order: Vec<String>,
    pub hidden: Vec<String>,
    pub favourites: Vec<String>,
}

impl Settings {
    pub fn arrangement(&self) -> Arrangement {
        Arrangement { order: self.order.clone(), hidden: self.hidden.clone(), favourites: self.favourites.clone() }
    }
}

/// What the island shows at rest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IslandRest {
    Robot,
    Clock,
    Battery,
}

impl IslandRest {
    pub const ALL: [IslandRest; 3] = [IslandRest::Robot, IslandRest::Clock, IslandRest::Battery];

    pub fn name(self) -> &'static str {
        match self {
            IslandRest::Robot => "robot",
            IslandRest::Clock => "clock",
            IslandRest::Battery => "battery",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == name)
    }

    pub fn step(self, step: i32) -> Self {
        let at = Self::ALL.iter().position(|v| *v == self).unwrap_or(0) as i32;
        Self::ALL[(at + step).rem_euclid(Self::ALL.len() as i32) as usize]
    }
}

impl Settings {
    /// The time as the clock settings have it.
    pub fn clock(&self, now: chrono::DateTime<chrono::Local>) -> String {
        let format = match (self.clock24, self.seconds) {
            (true, false) => "%H:%M",
            (true, true) => "%H:%M:%S",
            (false, false) => "%-I:%M",
            (false, true) => "%-I:%M:%S",
        };
        now.format(format).to_string()
    }
}

/// A greeting for the time of day, with the team when it is known.
pub fn greeting(hour: u32, team: Option<u32>) -> String {
    let part = match hour {
        5..=11 => "good morning",
        12..=17 => "good afternoon",
        18..=22 => "good evening",
        _ => "hello",
    };
    match team {
        Some(team) => format!("{part} · team {team}"),
        None => part.to_owned(),
    }
}

impl Default for Settings {
    fn default() -> Self {
        use crate::look::{Corners, Letters, Theme, Width};
        let theme = Theme::Catalyst;
        let (wallpaper, accent) = theme.defaults();
        Settings {
            light: false,
            calm: false,
            text: TextSize::Standard,
            accent,
            home: HomeStyle::Grid,
            open_robot: true,
            match_report: true,
            glance: vec![Widget::Device, Widget::Robot, Widget::NextMatch, Widget::LastMatch],
            pinned: None,
            theme,
            wallpaper,
            corners: Corners::Standard,
            letters: Letters::Crisp,
            width: Width::Normal,
            columns: 3,
            order: Vec::new(),
            hidden: Vec::new(),
            favourites: Vec::new(),
            clock24: true,
            seconds: false,
            island: IslandRest::Robot,
            greeting: true,
            robot_name: None,
            gpu: true,
            developer: false,
            motion_scale: 1.0,
            show_keys: false,
            toured: false,
        }
    }
}

/// A card the glance page can carry.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Widget {
    Device,
    Robot,
    NextMatch,
    LastMatch,
    Pinned,
    Note,
}

impl Widget {
    pub const ALL: [Widget; 6] = [Widget::Device, Widget::Robot, Widget::NextMatch, Widget::LastMatch, Widget::Pinned, Widget::Note];

    pub fn name(self) -> &'static str {
        match self {
            Widget::Device => "device",
            Widget::Robot => "robot",
            Widget::NextMatch => "next_match",
            Widget::LastMatch => "last_match",
            Widget::Pinned => "pinned",
            Widget::Note => "note",
        }
    }

    /// What Settings calls it.
    pub fn label(self) -> &'static str {
        match self {
            Widget::Device => "This handheld",
            Widget::Robot => "The robot",
            Widget::NextMatch => "Next match",
            Widget::LastMatch => "Last match",
            Widget::Pinned => "A pinned value",
            Widget::Note => "A note",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.name() == name)
    }

    /// The order a settings file lists, keeping each card once.
    pub fn list_from(text: &str) -> Vec<Self> {
        let mut out = Vec::new();
        for w in text.split(',').filter_map(|n| Self::from_name(n.trim())) {
            if !out.contains(&w) {
                out.push(w);
            }
        }
        out
    }
}

/// How Home lays the apps out.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HomeStyle {
    /// Tiles, six to a page.
    Grid,
    /// One large card at a time, turned through like a dial.
    Carousel,
}

impl HomeStyle {
    pub fn name(self) -> &'static str {
        match self {
            HomeStyle::Grid => "grid",
            HomeStyle::Carousel => "carousel",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [HomeStyle::Grid, HomeStyle::Carousel].into_iter().find(|style| style.name() == name)
    }

    pub fn next(self) -> Self {
        match self {
            HomeStyle::Grid => HomeStyle::Carousel,
            HomeStyle::Carousel => HomeStyle::Grid,
        }
    }
}

/// What a screen can reach while handling a key.
pub struct Cx<'a> {
    pub robot: &'a mut Robot,
    pub settings: &'a mut Settings,
    /// Set to show a message in the island.
    pub say: &'a mut Option<String>,
    /// No screen is on its way in or out.
    pub at_rest: bool,
}

pub enum Nav {
    Stay,
    /// A page deeper in the same app: slides in from the right.
    Push(Box<dyn Screen>),
    /// An app, opened from the tile at this rectangle: grows out of it.
    Open(Box<dyn Screen>, Rect),
    Pop,
    /// Pull the control centre down.
    Control,
    /// Done: give way to the screen beneath, fading out over it as it appears.
    Done,
    /// Back to Home, and take the tour there.
    Tour,
}

pub trait Screen {
    fn title(&self) -> &str;
    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav;
    fn draw(&mut self, g: &mut Gfx, robot: &Robot);
    /// True while a text field is being edited, so letters are not treated
    /// as navigation.
    fn wants_text(&self) -> bool {
        false
    }
    /// Advance the screen in time. True when it should be redrawn.
    fn tick(&mut self, _dt: f32) -> bool {
        false
    }
    /// True for a screen that should be looked at every frame beat even
    /// though nothing is moving, because something outside may change it at
    /// any moment (a shell printing). It is only redrawn when `tick` says so.
    fn busy(&self) -> bool {
        false
    }
    /// A word for the island when the screen opens.
    fn greeting(&self) -> Option<&'static str> {
        None
    }
    /// The screen is on top again after one above it closed.
    fn resume(&mut self) {}
    /// The screen is seen for the first time, now that the start animation
    /// is giving way to it: Home's tiles arrive then, not unseen beneath it.
    fn appear(&mut self) {}
    /// The screen takes the whole panel: no title, island or status over it.
    fn full_screen(&self) -> bool {
        false
    }
    /// Where along a strip of pages the screen is turned to, 0 at its left
    /// end and 1 at its right: the wallpaper follows a little (parallax).
    fn parallax(&self) -> Option<f32> {
        None
    }
    /// The screen has done its job and gives way to the one beneath, which
    /// appears as it fades.
    fn finished(&self) -> bool {
        false
    }
    /// Asked every frame while the screen is on top, with what a key would
    /// reach: for work that arrives from outside (a computer sending setup).
    fn update(&mut self, _cx: &mut Cx) -> Nav {
        Nav::Stay
    }
    /// An external app this screen wants the panel handed to. Asked once the
    /// screen has finished arriving; answered once.
    fn launch(&mut self) -> Option<crate::apps::App> {
        None
    }
    /// A robot has just linked while this screen is the one showing. Asked
    /// each frame until it answers with somewhere to go: Home turns to
    /// Robot's tile first, and then opens the app out of it.
    fn robot_linked(&mut self, _calm: bool) -> Nav {
        Nav::Stay
    }
    /// What the tour lights up when it shows the apps: the focused tile.
    fn spot(&self) -> Option<Rect> {
        None
    }
}

const MESSAGE_SECONDS: f32 = 2.4;

/// A link that was gone for less than this and came back is the same visit,
/// not a robot arriving: a loose cable must not keep opening the Robot app.
const LINK_GONE_SECONDS: f32 = 3.0;

/// The wallpaper's drift after a key: slow to start, slower to settle, no
/// overshoot. My own tuning (Bezel has no role for an ambient drift).
/// Response 1.6 s, critically damped: stiffness (2π/1.6)², damping 4π/1.6.
const DRIFT: Spring = Spring { mass: 1.0, stiffness: 15.42, damping: 7.854 };
/// How fast a key sets the wallpaper drifting, in pixels a second.
const DRIFT_PUSH: f32 = 26.0;

/// How long a robot's arrival waits for Home to be ready to open the app
/// (a transition to finish, Home to turn to the tile) before it is only
/// announced.
const ARRIVAL_PATIENCE: f32 = 2.0;

/// How long after a robot arrives a newly pulled log counts as the match it
/// has just come back from. A big log over the cable takes seconds; this
/// leaves room for a slow link and a log still being written.
const REPORT_WINDOW: f32 = 600.0;

/// After a robot arrives: the logs it had already, so the match it comes
/// back from can be told by its log being new.
struct ReportWatch {
    known: Vec<String>,
    waited: f32,
    /// A key has been pressed since: whoever pressed it is busy, and the
    /// report is announced rather than put in front of them.
    touched: bool,
    /// The logs' generation when last looked at; nothing is read until it moves.
    seen: u64,
}

/// How a screen arrived, which is also how it leaves.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Path {
    Slide,
    Grow(Rect),
}

impl Path {
    fn spring(&self) -> Spring {
        match self {
            // Bezel's page turn.
            Path::Slide => role::page(),
            // Bezel's app window: opening, closing and reopening.
            Path::Grow(_) => role::release(),
        }
    }
}

/// A finished frame of one screen, and the page dots it asked for.
type Picture = (Vec<u32>, Option<(usize, usize)>);

/// A screen on its way in or out. `progress` runs 0 (only the screen
/// underneath shows) to 1 (the arriving screen fills the panel). The two
/// screens are drawn once and then moved as pictures, so a frame of the
/// transition costs a copy rather than two redraws.
struct Transition {
    path: Path,
    progress: Animated,
    pictures: Option<[Picture; 2]>,
    /// Which taking of the pictures this is, so the GPU sends them up only
    /// when they change.
    made: u64,
    /// The screen on top while it leaves: already off the stack, kept until
    /// it has gone (or is asked back).
    leaving: Option<Box<dyn Screen>>,
}

pub struct Shell {
    pub gfx: Gfx,
    pub robot: Robot,
    pub settings: Settings,
    /// Each screen with the path it arrived by.
    stack: Vec<(Box<dyn Screen>, Path)>,
    transition: Option<Transition>,
    island_w: Animated,
    /// Where the island's width was when its words last changed, for fading them in.
    island_from: f32,
    island_words: String,
    message: Option<(String, f32)>,
    dirty: bool,
    /// Whether anything was in motion at the last tick. Time only passes for
    /// motion that was already under way: an animation begun by a key after
    /// an idle spell starts from its beginning, not a wake-up interval in.
    moving: bool,
    /// The ground and its glows, drawn once per palette and copied each frame.
    background: Vec<u32>,
    /// The window of the wallpaper the background was copied from.
    background_for: Option<(usize, usize)>,
    /// The wallpaper, larger than the panel, and what it was drawn for.
    wall: Vec<u32>,
    wall_for: Option<(u32, u32, u32, crate::look::Wallpaper)>,
    /// Where the wallpaper sits: Home's place along its pages (0 at the far
    /// left glance page, 1 at the last page of apps), which moves with Home's
    /// own page spring; and a drift that a key sets going and that settles
    /// back still.
    parallax: f32,
    drift: (Animated, Animated),
    /// The look last applied, to tell when it changes (and blend across).
    look_key: Option<(u32, u32, crate::look::Wallpaper, crate::look::Theme)>,
    status: Status,
    /// The top screen as it was last drawn, and how deep the stack was then:
    /// a transition starts from this picture instead of drawing it again.
    last_screen: Picture,
    last_depth: usize,
    /// The control centre, over whatever screen is showing.
    control: Control,
    /// While it is out: the frame it was pulled down over, and its sheet of
    /// glass as it looks at rest. The screen behind is held still, so the
    /// glass is worked out once and then moved as a picture.
    control_under: Option<Vec<u32>>,
    control_glass: Option<Vec<u32>>,
    /// Whether the robot was linked at the last tick, and how long it has
    /// been gone, to tell a robot arriving from a link that blinked.
    was_linked: bool,
    unlinked_for: f32,
    /// A robot has arrived and nothing has been done about it yet: for how long.
    arrival: Option<f32>,
    /// Watching for the log of the match a robot has come back from.
    report_watch: Option<ReportWatch>,
    /// The start animation, over everything until it has given way.
    intro: Option<crate::intro::Intro>,
    /// The last picture of a screen that has finished, fading out over the
    /// one it gave way to.
    curtain: Option<(Vec<u32>, Animated)>,
    /// Where drawing time has gone, for `--bench`: the screens themselves,
    /// and the glass over them.
    pub spent: (std::time::Duration, std::time::Duration),
    /// Room for a page rising in, kept between frames.
    rise_scratch: Vec<u32>,
    /// System updates: what the updater last said, when it was read, when
    /// the shell started, the last key, and the last check started.
    update: crate::sysupdate::Status,
    update_age: f32,
    started: std::time::Instant,
    said_well: bool,
    last_key: std::time::Instant,
    last_check: Option<std::time::Instant>,
    /// The tour of the keys, over everything while it shows; and, after
    /// first-boot setup, how long until it is offered (Home's tiles arrive first).
    tour: Option<crate::help::Tour>,
    tour_after: Option<f32>,
    /// Where the island and the page dots were last drawn, for the tour to light up.
    places: crate::help::Places,
    /// Developer options › Show key presses: the last few keys, and how long each has shown.
    echo: Vec<(String, f32)>,
}

/// How long a key shown by Show key presses stays, the last part fading.
const ECHO_SECONDS: f32 = 1.6;

/// A key as Show key presses names it.
fn key_name(key: Key) -> String {
    match key {
        Key::Up => "↑".into(),
        Key::Down => "↓".into(),
        Key::Left => "←".into(),
        Key::Right => "→".into(),
        Key::Enter => "enter".into(),
        Key::Back => "esc".into(),
        Key::Backspace => "bksp".into(),
        Key::Tab => "tab".into(),
        Key::PageUp => "pg up".into(),
        Key::PageDown => "pg dn".into(),
        Key::Char(' ') => "space".into(),
        Key::Char(c) => c.to_string(),
        Key::Ctrl(' ') => "ctrl space".into(),
        Key::Ctrl(c) => format!("ctrl {c}"),
        Key::Leave => "esc held".into(),
    }
}

/// The clock, battery and links at the top right, re-read now and then.
#[derive(Default)]
struct Status {
    clock: String,
    battery: Option<u32>,
    wired: bool,
    wireless: bool,
    age: f32,
}

impl Shell {
    pub fn new(robot: Robot) -> Self {
        // A robot already there (the demo one) has not just arrived; one that
        // links for the first time after start has, however soon.
        let linked = robot.linked();
        Shell {
            gfx: Gfx::new(Tokens::dark()),
            robot,
            settings: Settings::default(),
            was_linked: linked,
            unlinked_for: if linked { 0.0 } else { f32::INFINITY },
            arrival: None,
            report_watch: None,
            intro: None,
            curtain: None,
            stack: vec![(Box::new(Home::new()), Path::Slide)],
            transition: None,
            island_w: Animated::new(60.0, role::release()).with_rest(0.2),
            island_from: 60.0,
            island_words: String::new(),
            message: None,
            dirty: true,
            moving: false,
            background: Vec::new(),
            background_for: None,
            wall: Vec::new(),
            wall_for: None,
            parallax: 0.5,
            // At rest once under a third of a pixel: the window moves in whole pixels.
            drift: (Animated::new(0.0, DRIFT).with_rest(0.3), Animated::new(0.0, DRIFT).with_rest(0.3)),
            look_key: None,
            status: Status { age: f32::MAX, ..Status::default() },
            last_screen: (Vec::new(), None),
            rise_scratch: Vec::new(),
            update: crate::sysupdate::Status::default(),
            update_age: 0.0,
            started: std::time::Instant::now(),
            said_well: false,
            last_key: std::time::Instant::now(),
            last_check: None,
            last_depth: usize::MAX,
            control: Control::new(),
            control_under: None,
            control_glass: None,
            spent: Default::default(),
            tour: None,
            tour_after: None,
            places: crate::help::Places::default(),
            echo: Vec::new(),
        }
    }

    /// Take the tour now, over whatever is showing (Home, after setup or from
    /// Settings › Help).
    pub fn start_tour(&mut self) {
        self.tour = Some(crate::help::Tour::new(self.settings.calm));
        self.tour_after = None;
        self.moving = true;
        self.dirty = true;
    }

    /// Whether the tour is showing, and which step it is on.
    pub fn tour_step(&self) -> Option<usize> {
        self.tour.as_ref().map(|t| t.step())
    }

    /// Settings › Help › Take the tour: back to Home (the screens in between
    /// fade out over it), and the tour there.
    fn tour_from_home(&mut self) {
        if self.stack.len() > 1 {
            if !self.settings.calm {
                let mut fade = Animated::new(1.0, role::smooth()).with_rest(0.004);
                fade.set(0.0);
                self.curtain = Some((self.gfx.canvas.px.clone(), fade));
            }
            self.stack.truncate(1);
            self.transition = None;
            self.gfx.focus_reset();
            if let Some((home, _)) = self.stack.last_mut() {
                home.resume();
            }
        }
        self.start_tour();
    }

    /// How much slower motion runs: developer options' animation scale.
    fn motion_scale(&self) -> f32 {
        if self.settings.developer {
            self.settings.motion_scale.clamp(0.25, 10.0)
        } else {
            1.0
        }
    }

    /// Pull the control centre down, or put it away.
    pub fn set_control(&mut self, open: bool) {
        if open && !self.control.visible() {
            // Taken afresh on the next frame.
            self.control_under = None;
        }
        self.control.set_open(open, &self.settings);
        self.dirty = true;
    }

    #[cfg(test)]
    pub fn control_open(&self) -> bool {
        self.control.is_open()
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), MESSAGE_SECONDS));
        self.dirty = true;
    }

    /// Play the start animation, with `caption` under the name.
    pub fn start_intro(&mut self, caption: impl Into<String>) {
        self.intro = Some(crate::intro::Intro::new(caption));
        self.moving = true;
        self.dirty = true;
    }

    /// The title of the screen in front, for the screenshot tour and the
    /// pacing measurement.
    pub fn top_title(&self) -> &str {
        self.stack.last().map_or("", |(screen, _)| screen.title())
    }

    /// First start: setup, over Home, for the start animation to give way to.
    pub fn start_setup(&mut self) {
        self.stack.push((Box::new(crate::setup::Setup::new()), Path::Slide));
        self.dirty = true;
    }

    /// The screen on top has finished: it fades out over the one beneath,
    /// which appears (Home's tiles arrive) as it goes.
    fn finish_top(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let (finished, _) = self.stack.pop().expect("more than Home");
        // First-boot setup done: the tour is offered once Home has arrived.
        if finished.title() == "Setup" && self.stack.len() == 1 && !self.settings.toured {
            self.tour_after = Some(1.2);
        }
        self.transition = None;
        self.gfx.focus_reset();
        // What it changed (setup's look, from the handheld or a computer)
        // shows on what it gives way to.
        self.apply_settings();
        if let Some((screen, _)) = self.stack.last_mut() {
            screen.resume();
            screen.appear();
        }
        if self.stack.len() == 1 {
            self.greet();
        }
        if !self.settings.calm {
            let mut fade = Animated::new(1.0, role::smooth()).with_rest(0.004);
            fade.set(0.0);
            self.curtain = Some((self.gfx.canvas.px.clone(), fade));
        }
        self.moving = true;
        self.dirty = true;
    }

    /// The start animation gives way: what is underneath appears as it goes.
    fn end_intro(&mut self) {
        if let Some(intro) = &mut self.intro {
            if !intro.leaving() {
                intro.leave();
                if let Some((screen, _)) = self.stack.last_mut() {
                    screen.appear();
                }
                if self.stack.len() == 1 {
                    self.greet();
                }
            }
        }
    }

    /// Good morning, in the island, as Home appears (when the greeting is on).
    fn greet(&mut self) {
        if self.settings.greeting {
            use chrono::Timelike;
            let team = crate::schedule::team_of(&self.robot.address);
            self.say(greeting(chrono::Local::now().hour(), team));
        }
    }

    pub fn key(&mut self, key: Key) {
        self.dirty = true;
        self.last_key = std::time::Instant::now();
        // Developer options › Show key presses: every key, as it lands.
        if self.settings.developer && self.settings.show_keys {
            self.echo.push((key_name(key), 0.0));
            if self.echo.len() > 4 {
                self.echo.remove(0);
            }
            self.moving = true;
        }
        // A key during the start animation skips it, and is spent doing so;
        // once it is on its way out, keys go to the screen beneath at once.
        if let Some(intro) = &self.intro {
            if !intro.leaving() {
                return self.end_intro();
            }
            self.intro = None;
        }
        // The tour takes every key while it shows.
        if let Some(tour) = &mut self.tour {
            tour.key(key);
            self.moving = true;
            return;
        }
        // Whoever is pressing keys is busy: a robot that arrived a moment
        // ago does not take the screen from under them.
        self.arrival = None;
        if let Some(watch) = &mut self.report_watch {
            watch.touched = true;
        }
        // The Cardputer's arrows are Fn + F/X/Z/C. Outside text entry the bare
        // keys work too, so navigating never needs a modifier.
        let plain = |key: Key| match key {
            Key::Char('f') => Key::Up,
            Key::Char('x') => Key::Down,
            Key::Char('z') => Key::Left,
            Key::Char('c') => Key::Right,
            Key::Backspace => Key::Back,
            other => other,
        };
        // A key while the control centre is on its way out: it finishes going
        // at once, so the key lands on a screen that is drawn live, not on
        // the still picture held behind the sheet.
        if !self.control.is_open() && self.control.visible() {
            self.control.shown.jump(0.0);
            self.control_under = None;
            self.control_glass = None;
        }
        // The control centre takes the keys while it is down.
        if self.control.is_open() {
            match self.control.key(plain(key), &mut self.settings) {
                None => self.set_control(false),
                Some(Outcome::Settings) => {
                    self.apply_settings();
                    crate::save_settings(&self.settings);
                    // The screen behind changed its look: take it again.
                    self.control_under = None;
                }
                Some(Outcome::Nothing) => {}
            }
            return;
        }
        // Ctrl-Space brings it down over any screen, a terminal included.
        if key == Key::Ctrl(' ') {
            return self.set_control(true);
        }
        // ? brings the shortcut list up over any screen that is not taking
        // text (in a terminal or a text field, it types).
        if key == Key::Char('?') {
            let (top, _) = self.stack.last().expect("the home screen is always there");
            if !top.wants_text() && !top.full_screen() && top.title() != "Shortcuts" {
                return self.open(Box::new(crate::help::Shortcuts::new()), Path::Slide);
            }
        }
        // The living wallpaper: a move sets its light drifting the same way,
        // and it settles back still.
        if !self.settings.calm {
            let push = match plain(key) {
                Key::Left => (-DRIFT_PUSH, 0.0),
                Key::Right => (DRIFT_PUSH, 0.0),
                Key::Up => (0.0, -DRIFT_PUSH * 0.5),
                Key::Down => (0.0, DRIFT_PUSH * 0.5),
                _ => (0.0, 0.0),
            };
            if push != (0.0, 0.0) {
                self.drift.0.nudge(push.0);
                self.drift.1.nudge(push.1);
            }
        }
        let (top, _) = self.stack.last_mut().expect("the home screen is always there");
        let key = if top.wants_text() { key } else { plain(key) };
        // Enter answers as it lands: what it pressed gives a little.
        if key == Key::Enter && self.transition.is_none() {
            self.gfx.pressed();
        }
        let mut say = None;
        let at_rest = self.transition.is_none();
        let nav = top.key(key, &mut Cx { robot: &mut self.robot, settings: &mut self.settings, say: &mut say, at_rest });
        if let Some(text) = say {
            self.say(text);
        }
        self.follow(nav);
        self.apply_settings();
    }

    /// Go where a screen asked to.
    fn follow(&mut self, nav: Nav) {
        match nav {
            Nav::Stay => {}
            Nav::Push(screen) => self.open(screen, Path::Slide),
            Nav::Open(screen, from) => self.open(screen, Path::Grow(from)),
            Nav::Pop => self.close(),
            Nav::Control => self.set_control(true),
            Nav::Done => self.finish_top(),
            Nav::Tour => self.tour_from_home(),
        }
    }

    /// Bring the drawing in line with the settings: palette, calm, text size.
    pub fn apply_settings(&mut self) {
        self.dirty = true;
        self.gfx.accent = self.settings.accent;
        self.gfx.tokens = Tokens::of(self.settings.accent, if self.settings.light { Tone::Light } else { Tone::Dark });
        self.gfx.calm = self.settings.calm;
        self.gfx.home = self.settings.home;
        self.gfx.open_robot = self.settings.open_robot;
        self.gfx.match_report = self.settings.match_report;
        self.gfx.glance.clone_from(&self.settings.glance);
        self.gfx.pinned.clone_from(&self.settings.pinned);
        let arrangement = self.settings.arrangement();
        if self.gfx.arrangement != arrangement {
            self.gfx.arrangement = arrangement;
        }
        if self.gfx.clock != (self.settings.clock24, self.settings.seconds) {
            // The clock in the new format at once, not at its next reading.
            self.gfx.clock = (self.settings.clock24, self.settings.seconds);
            self.status.clock = self.settings.clock(chrono::Local::now());
        }
        self.gfx.island = self.settings.island;
        self.gfx.greeting = self.settings.greeting;
        (self.gfx.developer, self.gfx.motion_scale, self.gfx.show_keys) = (self.settings.developer, self.settings.motion_scale, self.settings.show_keys);
        self.gfx.want_gpu(self.settings.gpu);
        let s = &self.settings;
        let look = crate::look::Look { theme: s.theme, wallpaper: s.wallpaper, corners: s.corners, letters: s.letters, width: s.width };
        self.gfx.set_look(look, s.columns);
        self.gfx.set_text_size(s.text);
        // A new look (accent, mode, theme, wallpaper) blends in over the old
        // one rather than snapping to it.
        let t = self.gfx.tokens;
        let key = (t.ground.to_u32(), t.signal.to_u32(), s.wallpaper, s.theme);
        if self.look_key.is_some_and(|old| old != key) && !s.calm && self.intro.is_none() {
            let mut fade = Animated::new(1.0, role::smooth()).with_rest(0.004);
            fade.set(0.0);
            self.curtain = Some((self.gfx.canvas.px.clone(), fade));
            self.control_under = None;
            self.moving = true;
        }
        self.look_key = Some(key);
    }

    /// Where the panel's window into the wallpaper is: parallax across
    /// Home's pages and the drift a key set going, both off in the calm version.
    fn wall_at(&self) -> (usize, usize) {
        let (rx, ry) = crate::look::REACH;
        if self.settings.calm {
            return (rx, ry);
        }
        let x = self.parallax * 2.0 * rx as f32 + self.drift.0.value();
        let y = ry as f32 + self.drift.1.value();
        (x.round().clamp(0.0, 2.0 * rx as f32) as usize, y.round().clamp(0.0, 2.0 * ry as f32) as usize)
    }

    fn open(&mut self, screen: Box<dyn Screen>, path: Path) {
        self.gfx.focus_reset();
        if let Some(words) = screen.greeting() {
            self.say(words);
        }
        // Asked back while it was leaving by the same path: it returns from
        // where it is, and the new copy is not needed. Only the same screen:
        // Back, Down, Enter quickly is a different page by the same path.
        if let Some(t) = &mut self.transition {
            if t.path == path && t.leaving.as_ref().is_some_and(|leaving| leaving.title() == screen.title()) {
                let returning = t.leaving.take().expect("checked above");
                self.stack.push((returning, path));
                t.progress.set(1.0);
                return;
            }
        }
        self.stack.push((screen, path));
        self.gfx.reveal_start();
        if self.settings.calm {
            self.transition = None;
            return;
        }
        let mut progress = Animated::new(0.0, path.spring()).with_rest(0.002);
        progress.set(1.0);
        self.transition = Some(Transition { path, progress, pictures: None, made: 0, leaving: None });
    }

    fn close(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        self.gfx.focus_reset();
        self.gfx.reveal_stop();
        let (screen, path) = self.stack.pop().expect("more than one screen");
        if let Some((below, _)) = self.stack.last_mut() {
            below.resume();
        }
        if self.settings.calm {
            self.transition = None;
            return;
        }
        match &mut self.transition {
            // Still arriving: turn it round where it is, keeping its speed.
            Some(t) if t.leaving.is_none() => {
                t.leaving = Some(screen);
                t.progress.set(0.0);
            }
            _ => {
                let mut progress = Animated::new(1.0, path.spring()).with_rest(0.002);
                progress.set(0.0);
                self.transition = Some(Transition { path, progress, pictures: None, made: 0, leaving: Some(screen) });
            }
        }
    }

    /// An external app to hand the panel to: the top screen asks for one, and
    /// has finished arriving (so the app starts on a settled picture).
    pub fn take_launch(&mut self) -> Option<crate::apps::App> {
        if self.transition.is_some() {
            return None;
        }
        self.stack.last_mut().and_then(|(screen, _)| screen.launch())
    }

    /// The app that had the panel is done: say so if it failed, and close
    /// the page it was opened from.
    pub fn launch_ended(&mut self, result: Result<(), String>) {
        if let Err(why) = result {
            self.say(why);
        }
        self.close();
        self.dirty = true;
    }

    /// Notice a robot linking. With Home showing (and the setting on) the
    /// Robot app opens out of its tile; anywhere else the island says so and
    /// the screen in front is left alone.
    fn watch_link(&mut self, dt: f32) {
        if !self.robot.linked() {
            if self.was_linked {
                self.unlinked_for = 0.0;
            }
            self.unlinked_for += dt;
            self.was_linked = false;
            self.arrival = None;
            self.report_watch = None;
            return;
        }
        if !self.was_linked {
            self.was_linked = true;
            if self.unlinked_for >= LINK_GONE_SECONDS {
                self.arrival = Some(0.0);
                let known = self.robot.logs.list().into_iter().map(|row| row.name).collect();
                self.report_watch = Some(ReportWatch { known, waited: 0.0, touched: false, seen: self.robot.logs.generation() });
            }
        }
        self.remember_name();
        self.watch_report(dt);
        let Some(waited) = self.arrival else { return };
        let patient = waited < ARRIVAL_PATIENCE;
        // A screen on its way in or out is let finish first.
        if self.transition.is_some() && patient {
            self.arrival = Some(waited + dt);
            return;
        }
        let on_home = self.stack.len() == 1 && self.transition.is_none() && !self.control.visible();
        if self.settings.open_robot && on_home && patient {
            let calm = self.settings.calm;
            self.dirty = true;
            match self.stack[0].0.robot_linked(calm) {
                // Home is turning to Robot's tile: ask again next frame.
                Nav::Stay => return self.arrival = Some(waited + dt),
                nav => self.follow(nav),
            }
            self.arrival = None;
            return;
        }
        self.arrival = None;
        // Not over something the island is already saying.
        if self.message.is_none() {
            let name = self.robot.string("Robot/Identity/Name").unwrap_or_else(|| "robot".into()).to_lowercase();
            self.say(format!("{name} linked"));
        }
    }

    /// System updates, every few seconds: say the shell is well (which keeps
    /// a system tried after an update), look for updates once a day when
    /// online, download one when it is found (unless that is switched off),
    /// say when an update reaches a new stage, and restart into a waiting one
    /// at three in the morning if asked to and nobody is using the handheld.
    fn watch_update(&mut self, dt: f32) {
        use crate::sysupdate;
        if !self.said_well && self.started.elapsed().as_secs() >= 15 {
            sysupdate::say_well();
            self.said_well = true;
        }
        self.update_age += dt;
        if self.update_age < 5.0 || !sysupdate::available() {
            return;
        }
        self.update_age = 0.0;
        let s = sysupdate::read();
        let stage = format!("{}:{}", s.state, s.version);
        if s != self.update && sysupdate::pref("announced", "") != stage {
            if let Some(words) = sysupdate::notice(&s) {
                self.say(words);
            }
            sysupdate::set_pref("announced", &stage);
            if s.state == "available" && sysupdate::pref("auto", "on") == "on" {
                sysupdate::run("install").ok();
            }
        }
        let online = self.status.wired || self.status.wireless;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let due = now.saturating_sub(s.checked) > 24 * 3600 && self.last_check.is_none_or(|t| t.elapsed().as_secs() > 3600);
        if online && due && self.started.elapsed().as_secs() > 90 && !matches!(s.state.as_str(), "downloading" | "installing" | "trying") {
            self.last_check = Some(std::time::Instant::now());
            sysupdate::run("check").ok();
        }
        use chrono::Timelike;
        let idle = self.last_key.elapsed().as_secs() > 600;
        if s.state == "ready" && sysupdate::pref("overnight", "off") == "on" && chrono::Local::now().hour() == 3 && idle {
            sysupdate::run("apply").ok();
        }
        self.update = s;
    }

    /// The linked robot's name, kept for the start screen ("team 5805 ·
    /// ferrum"). Not the demo robot's.
    fn remember_name(&mut self) {
        if self.robot.is_demo() {
            return;
        }
        let Some(name) = self.robot.string("Robot/Identity/Name").map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty() && n.len() <= 32) else { return };
        if self.settings.robot_name.as_deref() != Some(name.as_str()) {
            self.settings.robot_name = Some(name);
            crate::save_settings(&self.settings);
        }
    }

    /// A robot back from a match: once that match's log has been pulled and
    /// read, show its summary. Only over Home or the Robot page that opened
    /// by itself, and only if nobody has pressed a key since; otherwise the
    /// island says where to find it.
    fn watch_report(&mut self, dt: f32) {
        let Some(watch) = &mut self.report_watch else { return };
        watch.waited += dt;
        if watch.waited > REPORT_WINDOW || !self.settings.match_report {
            self.report_watch = None;
            return;
        }
        // The Robot app is still on its way: the report waits its turn.
        let generation = self.robot.logs.generation();
        if self.arrival.is_some() || generation == watch.seen {
            return;
        }
        watch.seen = generation;
        // Newest first: the first log not here before is the match just played.
        let rows = self.robot.logs.list();
        let Some(row) = rows.iter().find(|row| !watch.known.contains(&row.name)) else { return };
        let Some(detail) = crate::screens::LogDetail::of(row) else { return };
        let busy = watch.touched || self.control.visible() || self.transition.is_some();
        let over = match self.stack.len() {
            1 => true,
            2 => self.stack[1].0.title() == "Robot",
            _ => false,
        };
        // Just the match ("nyro q14"), not its date: the island stays short.
        let title = row.title();
        let short = title.split(" · ").next().unwrap_or(&title).to_owned();
        self.report_watch = None;
        if busy || !over {
            self.say(format!("{short} summary is in Logs"));
            return;
        }
        self.say(format!("how {short} went"));
        self.open(Box::new(detail), Path::Slide);
    }

    /// Advance time. Returns whether a frame should be drawn.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.gfx.tick_gpu();
        self.watch_link(dt);
        // Motion only moves forward in time if it was already moving; slower
        // under developer options' animation scale.
        let motion_dt = if self.moving { dt.min(0.05) / self.motion_scale() } else { 0.0 };
        let mut moving = false;
        // The tour: offered a moment after setup, once Home has arrived.
        if let Some(left) = &mut self.tour_after {
            *left -= dt;
            if *left <= 0.0 && self.transition.is_none() && self.curtain.is_none() && self.stack.len() == 1 && self.intro.is_none() {
                self.start_tour();
            }
        }
        if let Some(tour) = &mut self.tour {
            moving |= tour.tick(motion_dt);
            if tour.gone() {
                self.tour = None;
                self.settings.toured = true;
                crate::save_settings(&self.settings);
                self.dirty = true;
            }
        }
        // Keys shown by Show key presses fade on the clock, not the motion.
        if !self.echo.is_empty() {
            for (_, age) in &mut self.echo {
                *age += dt;
            }
            self.echo.retain(|(_, age)| *age < ECHO_SECONDS);
            moving = true;
            if self.echo.is_empty() {
                self.dirty = true;
            }
        }
        if let Some(t) = &mut self.transition {
            if t.progress.tick(motion_dt) || motion_dt == 0.0 {
                moving = true;
            } else {
                let ended = self.transition.take().expect("in flight");
                // The living wallpaper: once a screen has come or gone, its
                // light drifts the way it went, and settles still.
                if !self.settings.calm {
                    let closing = ended.leaving.is_some();
                    let push = match (ended.path, closing) {
                        (Path::Slide, false) => (DRIFT_PUSH * 0.7, 0.0),
                        (Path::Slide, true) => (-DRIFT_PUSH * 0.7, 0.0),
                        (Path::Grow(_), false) => (0.0, -DRIFT_PUSH * 0.5),
                        (Path::Grow(_), true) => (0.0, DRIFT_PUSH * 0.5),
                    };
                    self.drift.0.nudge(push.0);
                    self.drift.1.nudge(push.1);
                    moving = true;
                }
            }
        }
        if let Some(intro) = &mut self.intro {
            moving |= intro.tick(motion_dt);
            if intro.finished() && !intro.leaving() {
                self.end_intro();
            } else if intro.gone() {
                self.intro = None;
                self.dirty = true;
            }
        }
        moving |= self.island_w.tick(motion_dt);
        moving |= self.gfx.tick(motion_dt);
        // The drift holds still while a screen comes or goes: the transition's
        // pictures were taken over the wallpaper where it was, and moving it
        // would mean taking them again every frame.
        if self.transition.is_none() {
            moving |= self.drift.0.tick(motion_dt) | self.drift.1.tick(motion_dt);
        }
        moving |= self.control.tick(motion_dt);
        if !self.control.visible() && self.control_under.is_some() {
            // Put away: let go of its pictures, and draw the screen afresh.
            self.control_under = None;
            self.control_glass = None;
            self.dirty = true;
        }
        if self.transition.is_none() {
            moving |= self.stack.last_mut().is_some_and(|(s, _)| s.tick(motion_dt));
            // Work that reached the screen from outside, and a screen that is done.
            let mut say = None;
            let nav = {
                let (top, _) = self.stack.last_mut().expect("the home screen is always there");
                top.update(&mut Cx { robot: &mut self.robot, settings: &mut self.settings, say: &mut say, at_rest: true })
            };
            if let Some(text) = say {
                self.say(text);
            }
            if !matches!(nav, Nav::Stay) {
                self.follow(nav);
                self.apply_settings();
                moving = true;
            }
            if self.stack.last().is_some_and(|(s, _)| s.finished()) {
                self.finish_top();
                moving = true;
            }
        }
        if let Some((_, fade)) = &mut self.curtain {
            moving |= fade.tick(motion_dt);
            if fade.value() <= 0.004 {
                self.curtain = None;
                self.dirty = true;
            }
        }
        if let Some((_, left)) = &mut self.message {
            *left -= dt;
            if *left <= 0.0 {
                self.message = None;
                self.dirty = true;
            }
        }
        // The clock and the battery, a few times a minute (every second
        // while the clock shows seconds).
        self.status.age += dt;
        if self.status.age > if self.settings.seconds { 1.0 } else { 5.0 } {
            self.status.age = 0.0;
            self.gfx.look_for_gpu();
            let clock = self.settings.clock(chrono::Local::now());
            let battery = crate::sys::battery_percent();
            let (wired, wireless) = (crate::sys::link_up("eth0"), crate::sys::link_up("wlan0"));
            if (clock.as_str(), battery, wired, wireless) != (self.status.clock.as_str(), self.status.battery, self.status.wired, self.status.wireless) {
                self.status = Status { clock, battery, wired, wireless, age: 0.0 };
                self.dirty = true;
            }
        }
        self.watch_update(dt);
        let changed = self.robot.refresh(dt);
        let draw = moving || changed || self.dirty;
        self.dirty = false;
        self.moving = moving;
        draw
    }

    /// Whether something on screen is in motion.
    pub fn moving(&self) -> bool {
        self.moving
    }

    /// Whether to keep to the frame beat: something is moving, or the
    /// screen in front wants looking at often.
    pub fn animating(&self) -> bool {
        self.moving || self.stack.last().is_some_and(|(s, _)| s.busy())
    }

    /// How far the transition in flight has come, if there is one.
    #[cfg(test)]
    fn progress(&self) -> Option<f32> {
        self.transition.as_ref().map(|t| t.progress.value())
    }

    pub fn draw(&mut self) {
        // While the start animation covers the panel, nothing beneath is drawn.
        if let Some(intro) = &self.intro {
            if !intro.leaving() {
                intro.draw(&mut self.gfx);
                return;
            }
        }
        self.draw_under_intro();
        // A finished screen's last picture, fading out over what it gave way to.
        if let Some((picture, fade)) = &self.curtain {
            let f = (fade.value().clamp(0.0, 1.0) * 256.0) as u32;
            bezel::simd::mix_into(&mut self.gfx.canvas.px, Some(picture), f);
        }
        if let Some(intro) = &self.intro {
            intro.draw(&mut self.gfx);
        }
        // The tour, over everything.
        if let Some(tour) = &mut self.tour {
            let at_home = self.stack.len() == 1 && self.transition.is_none();
            let tile = if at_home { self.stack.last().and_then(|(s, _)| s.spot()) } else { None };
            let places = crate::help::Places { tile, ..self.places };
            tour.draw(&mut self.gfx, &places);
            if self.gfx.take_motion_started() {
                self.moving = true;
            }
        }
        self.draw_echo();
    }

    /// Developer options › Show key presses: the last keys, each on a cap,
    /// along the bottom of the screen, fading as they age.
    fn draw_echo(&mut self) {
        if self.echo.is_empty() {
            return;
        }
        let g = &mut self.gfx;
        let t = g.tokens;
        let style = g.ty.label;
        let cap_h = style.size + 8.0;
        let widths: Vec<f32> = self.echo.iter().map(|(name, _)| g.measure(name, style) + 12.0).collect();
        let total = widths.iter().sum::<f32>() + 4.0 * (widths.len() - 1) as f32;
        let mut x = ((W - total) * 0.5).round();
        let y = H - 34.0;
        for ((name, age), w) in self.echo.iter().zip(widths) {
            let alpha = ((ECHO_SECONDS - age) / 0.4).clamp(0.0, 1.0);
            let cap = Rect::new(x, y, w, cap_h);
            g.shadow(cap, 5.0, alpha);
            g.canvas.rrect(cap, 5.0, t.surface3, alpha);
            g.text.draw_aligned(&mut g.canvas, cap.cx(), (cap.cy() + style.size * 0.36).round(), Align::Center, name, &style, t.ink, alpha);
            x += w + 4.0;
        }
    }

    fn draw_under_intro(&mut self) {
        let over = self.control.visible();
        match (&self.control_under, over) {
            // The control centre is out: the screen behind is held as it was.
            (Some(under), true) => self.gfx.canvas.px.copy_from_slice(under),
            _ => {
                self.draw_frame();
                if over {
                    self.control_under = Some(self.gfx.canvas.px.clone());
                    self.control_glass = None;
                }
            }
        }
        if over {
            self.draw_control();
        }
    }

    /// The control centre over the frame: the screen dims as the sheet comes
    /// down, the glass is moved to where the sheet is, and the switches are
    /// drawn on it.
    fn draw_control(&mut self) {
        const DIM: f32 = 0.45;
        let Shell { gfx: g, control, control_glass, .. } = self;
        let dim = |px: &mut [u32], amount: f32| {
            let f = (amount.clamp(0.0, 1.0) * DIM * 256.0) as u32;
            bezel::simd::mix_into(px, None, f);
        };
        if control_glass.is_none() {
            // The glass as it will rest: refracting the dimmed screen behind it.
            let frame = g.canvas.px.clone();
            dim(&mut g.canvas.px, 1.0);
            g.glass(&[GlassShape::new(SHEET, SHEET_RADIUS, 2)]);
            *control_glass = Some(std::mem::replace(&mut g.canvas.px, frame));
        }
        dim(&mut g.canvas.px, control.shown.value());
        blit_sheet(&mut g.canvas.px, control_glass.as_ref().expect("made above"), control.offset());
        control.draw(g);
        if g.take_motion_started() {
            self.moving = true;
        }
    }

    fn draw_frame(&mut self) {
        let began = std::time::Instant::now();
        let t = self.gfx.tokens;
        // Home's place along its pages, which the wallpaper follows.
        if let ([(home, _)], None) = (self.stack.as_slice(), &self.transition) {
            if let Some(at) = home.parallax() {
                self.parallax = at;
            }
        }
        // The wallpaper: drawn once for each palette and kind, larger than
        // the panel; each frame shows the window of it that parallax and
        // drift have moved to.
        let wall_key = (t.ground.to_u32(), t.signal.to_u32(), t.ice.to_u32(), self.settings.wallpaper);
        if self.wall_for != Some(wall_key) {
            self.wall = crate::look::wallpaper(self.settings.wallpaper, &t);
            self.wall_for = Some(wall_key);
            self.background_for = None;
        }
        let at = self.wall_at();
        let g = &mut self.gfx;
        if self.background_for != Some(at) {
            self.background.resize(g.canvas.px.len(), 0);
            crate::look::window(&self.wall, at.0, at.1, &mut self.background);
            self.background_for = Some(at);
            if let Some(transition) = &mut self.transition {
                transition.pictures = None;
            }
        }
        g.canvas.px.copy_from_slice(&self.background);
        g.pager = None;
        // A title runs up to the island, however wide the island is just now.
        g.title_room = (W - self.island_w.value().max(g.island_h())) * 0.5 - PAD - 8.0;
        g.ox = 0.0;

        let depth = self.stack.len();
        if let Some(transition) = &mut self.transition {
            // Take the two pictures on the transition's first frame.
            if transition.pictures.is_none() {
                g.focus_static = true;
                let scratch = &mut self.rise_scratch;
                let mut picture = |g: &mut Gfx, screen: &mut dyn Screen, home: bool, rising: bool| -> Picture {
                    g.canvas.px.copy_from_slice(&self.background);
                    g.pager = None;
                    Self::draw_screen(g, screen, &self.robot, home);
                    // Only the page on its way in rises; one beneath is drawn as it rests.
                    if rising && g.revealing() && !screen.full_screen() {
                        rise_in(g, &self.background, scratch);
                    }
                    (g.canvas.px.clone(), g.pager.take().map(|(pages, current, _)| (pages, current)))
                };
                // The screen that was showing was drawn last frame. Keeping that
                // picture means a transition's first frame draws one screen,
                // not two, so it starts without a stall.
                let kept = |depth_then: usize| (self.last_depth == depth_then).then(|| self.last_screen.clone());
                transition.pictures = Some(match &mut transition.leaving {
                    Some(leaving) => {
                        let over = kept(depth + 1).unwrap_or_else(|| picture(g, leaving.as_mut(), false, false));
                        [picture(g, self.stack[depth - 1].0.as_mut(), depth == 1, false), over]
                    }
                    None => {
                        let (under, over) = self.stack.split_at_mut(depth - 1);
                        let under = kept(depth - 1).or_else(|| under.last_mut().map(|(s, _)| picture(g, s.as_mut(), depth == 2, false)));
                        [under.unwrap_or_else(|| (self.background.clone(), None)), picture(g, over[0].0.as_mut(), false, true)]
                    }
                });
                self.last_depth = usize::MAX;
                g.focus_static = false;
                g.canvas.px.copy_from_slice(&self.background);
                transition.made = PICTURES_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            } else if transition.leaving.is_none() && g.revealing() {
                // The page on its way in is still rising: draw it again.
                g.focus_static = true;
                g.pager = None;
                Self::draw_screen(g, self.stack[depth - 1].0.as_mut(), &self.robot, false);
                if !self.stack[depth - 1].0.full_screen() {
                    rise_in(g, &self.background, &mut self.rise_scratch);
                }
                let dots = g.pager.take().map(|(pages, current, _)| (pages, current));
                if let Some([_, over]) = &mut transition.pictures {
                    over.0.copy_from_slice(&g.canvas.px);
                    over.1 = dots;
                }
                transition.made = PICTURES_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                g.focus_static = false;
                g.canvas.px.copy_from_slice(&self.background);
            }
            let p = transition.progress.value();
            let [(under, under_dots), (over, over_dots)] = transition.pictures.as_ref().expect("taken above");
            match transition.path {
                Path::Slide => {
                    // The one underneath drifts a third as far as the one on top travels.
                    let (under_x, over_x) = ((-p * W * 0.3).round(), ((1.0 - p) * W).round());
                    blit_shifted(&mut g.canvas.px, under, W as usize, H as usize, under_x as i32);
                    blit_shifted(&mut g.canvas.px, over, W as usize, H as usize, over_x as i32);
                    g.pager = under_dots.map(|(pages, current)| (pages, current, under_x)).or(over_dots.map(|(pages, current)| (pages, current, over_x)));
                }
                Path::Grow(from) => {
                    // From the tile's place and corners to the whole panel.
                    let grown = p.clamp(0.0, 1.0);
                    let window = from.lerp(&Rect::new(0.0, 0.0, W, H), grown);
                    let corner = g.tile_radius() * (1.0 - grown);
                    // Small, the window is the tile it grows from; the app takes it over as it grows.
                    let morph = Morph { under, tile: from, screen: Morph::screen_at(grown) };
                    // On the GPU when it is drawing; otherwise, or if it fails, here.
                    let on_gpu = match &mut g.gpu {
                        Some(gpu) => gpu.grow(&mut g.canvas, under, over, transition.made, window, corner, t.ground, Some(&morph)),
                        None => Ok(false),
                    };
                    if let Err(why) = &on_gpu {
                        g.gpu_failed(why.clone());
                    }
                    if on_gpu != Ok(true) {
                        g.canvas.px.copy_from_slice(under);
                        blit_scaled(&mut g.canvas, over, window, corner, t.ground, Some(&morph));
                    }
                    // The page dots belong to the screen underneath, until the window covers them.
                    if window.bottom() < H - 14.0 {
                        g.pager = under_dots.map(|(pages, current)| (pages, current, 0.0));
                    }
                }
            }
        } else {
            Self::draw_screen(g, self.stack[depth - 1].0.as_mut(), &self.robot, depth == 1);
            // Still rising once its transition is over: the last blocks arrive live.
            if depth > 1 && g.revealing() && !self.stack[depth - 1].0.full_screen() {
                rise_in(g, &self.background, &mut self.rise_scratch);
            }
            self.last_screen.0.clone_from(&g.canvas.px);
            self.last_screen.1 = g.pager.map(|(pages, current, _)| (pages, current));
            self.last_depth = depth;
        }
        g.ox = 0.0;
        // A screen that takes the whole panel has no island or status over it.
        if self.stack.last().is_some_and(|(s, _)| s.full_screen()) {
            if g.take_motion_started() {
                self.moving = true;
            }
            return;
        }

        // Status at the top right: links, battery, the time.
        let ty = g.ty;
        let bar_cy = 4.0 + g.island_h() * 0.5;
        let mut x = W - PAD - 2.0;
        // What the island shows at rest is not shown twice.
        let rest = self.settings.island;
        if rest != IslandRest::Clock {
            x -= g.text_at(x, bar_cy, Align::Right, &self.status.clock, ty.caption, t.dim) + 7.0;
        }
        if let (Some(percent), false) = (self.status.battery, rest == IslandRest::Battery) {
            let ink = if percent <= 15 { g.fault() } else { t.dim };
            x -= g.text_at(x, bar_cy, Align::Right, &format!("{percent}%"), ty.caption, ink) + 7.0;
        }
        for (up, icon) in [(self.status.wireless, "wifi"), (self.status.wired, "lan")] {
            if up {
                g.icon(icon, false, x - 6.0, bar_cy, 13.0, t.dim);
                x -= 17.0;
            }
        }

        // The island: link status at rest, growing to carry a message.
        let linked = self.robot.linked();
        let (label, icon) = match (&self.message, self.settings.island) {
            (Some((text, _)), _) => (text.clone(), true),
            (None, IslandRest::Clock) => (self.status.clock.clone(), false),
            (None, IslandRest::Battery) => (self.status.battery.map_or_else(|| "battery –".to_owned(), |p| format!("battery {p}%")), false),
            (None, IslandRest::Robot) if linked => (self.robot.string("Robot/Identity/Name").unwrap_or_else(|| "robot".into()).to_lowercase(), false),
            (None, IslandRest::Robot) => (self.robot.status(), false),
        };
        let caption = g.ty.caption;
        let island_h = g.island_h();
        // A message may take most of the width; status stays out of the title's way.
        let room = if icon { 250.0 } else { 96.0 };
        let label = g.text.fit(&label, &caption, room);
        let want = g.measure(&label, caption) + 34.0;
        if label != self.island_words {
            self.island_words = label.clone();
            self.island_from = self.island_w.value();
        }
        if (want - self.island_w.target()).abs() > 0.5 {
            // Growing arrives on the release spring; shrinking leaves on smooth.
            let growing = want > self.island_w.target();
            self.island_w.set_with(want, if growing { role::release() } else { role::smooth() });
            self.moving = true;
        }
        if self.settings.calm {
            self.island_w.jump(want);
        }
        let iw = self.island_w.value().max(island_h);
        let island = Rect::new((W - iw) * 0.5, 4.0, iw, island_h);
        self.places.island = Some(island);
        // Every glass shape goes through one pass; each family is its own group.
        let mut shapes = vec![GlassShape::new(island, island_h * 0.5, 0)];
        let pager = g.pager.map(|(pages, current, ox)| {
            let w = pages as f32 * 9.0 + 7.0;
            (Rect::new((W - w) * 0.5 + ox, H - 13.0, w, 10.0), pages, current)
        });
        if let Some((platter, ..)) = pager {
            shapes.push(GlassShape::new(platter, 5.0, 1));
        }
        self.places.pager = pager.map(|(platter, ..)| platter);
        let before_glass = std::time::Instant::now();
        g.glass(&shapes);
        self.spent.0 += before_glass - began;
        self.spent.1 += before_glass.elapsed();
        if let Some((platter, pages, current)) = pager {
            for i in 0..pages {
                let color = if i == current { t.ink } else { t.faint };
                g.canvas.circle(platter.x + 8.0 + i as f32 * 9.0, platter.cy(), 2.0, color, 1.0);
            }
        }

        // What sits on glass is not glass. New words fade in once the island
        // is past halfway (55%) to its new width.
        let travel = want - self.island_from;
        let grown = if travel.abs() < 1.0 { 1.0 } else { ((self.island_w.value() - self.island_from) / travel).clamp(0.0, 1.0) };
        let words = ((grown - 0.55) / 0.45).clamp(0.0, 1.0);
        g.canvas.set_clip(island.inset(2.0));
        let dot = if icon { t.signal } else if linked { t.ice } else { t.faint };
        g.canvas.circle(island.x + 12.0, island.cy(), 3.0, dot, 1.0);
        g.text_alpha(island.x + 22.0, island.cy(), &label, caption, t.ink, words);
        g.canvas.clear_clip();
        if g.take_motion_started() {
            self.moving = true;
        }
    }

    fn draw_screen(g: &mut Gfx, screen: &mut dyn Screen, robot: &Robot, _home: bool) {
        let t = g.tokens;
        let ty = g.ty;
        // The title shares the island's line; it gives way to a wide island.
        let bar_cy = 4.0 + g.island_h() * 0.5;
        let room = g.title_room;
        if !screen.full_screen() {
            // Bezel's headings are lowercase, as its panels' are.
            let title = match g.theme {
                crate::look::Theme::Bezel => screen.title().to_lowercase(),
                crate::look::Theme::Catalyst => screen.title().to_owned(),
            };
            g.text_fit(PAD + 2.0, bar_cy, &title, ty.title, t.ink, room);
        }
        screen.draw(g, robot);
    }
}

/// Copy a full-screen picture into the frame, moved sideways by `dx` pixels.
fn blit_shifted(dst: &mut [u32], src: &[u32], w: usize, h: usize, dx: i32) {
    let shift = dx.unsigned_abs() as usize;
    if shift >= w {
        return;
    }
    let (to, from) = if dx >= 0 { (shift, 0) } else { (0, shift) };
    let n = w - shift;
    for y in 0..h {
        dst[y * w + to..y * w + to + n].copy_from_slice(&src[y * w + from..y * w + from + n]);
    }
}

/// Copy the control centre's sheet out of the picture of it at rest, moved
/// down by `dy` pixels (negative: up, on its way in), keeping its rounded corners.
fn blit_sheet(dst: &mut [u32], src: &[u32], dy: f32) {
    let (w, h) = (W as i32, H as i32);
    let dy = dy.round() as i32;
    let (x0, x1) = (SHEET.x.floor() as i32, SHEET.right().ceil() as i32);
    for from_y in SHEET.y.floor() as i32..SHEET.bottom().ceil() as i32 {
        let y = from_y + dy;
        if y < 0 || y >= h {
            continue;
        }
        let fy = from_y as f32 + 0.5;
        let (from, to) = ((from_y * w) as usize, (y * w) as usize);
        // Only the rows that pass through a corner need the edge worked out.
        if fy > SHEET.y + SHEET_RADIUS && fy < SHEET.bottom() - SHEET_RADIUS {
            dst[to + x0 as usize..to + x1 as usize].copy_from_slice(&src[from + x0 as usize..from + x1 as usize]);
            continue;
        }
        for x in x0..x1 {
            let cover = (0.5 - sd_rrect(x as f32 + 0.5, fy, &SHEET, SHEET_RADIUS)).clamp(0.0, 1.0);
            if cover > 0.0 {
                dst[to + x as usize] = mix_px(dst[to + x as usize], src[from + x as usize], (cover * 256.0) as u32);
            }
        }
    }
}

use bezel::simd::mix_px;

/// Pages rise in. The arriving page, as drawn, is cut into blocks at the
/// rows where nothing but the wallpaper shows (the scrollbar's strip at the
/// right aside), so a block is a title, a row, a line of tiles; never half
/// of anything. Each block arrives a little after the one above it, rising
/// into place, and is not there before its turn. Only what the page drew
/// moves: the wallpaper behind stays where it is.
fn rise_in(g: &mut Gfx, background: &[u32], scratch: &mut Vec<u32>) {
    let (w, h) = (W as usize, H as usize);
    let mut px = std::mem::take(&mut g.canvas.px);
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
        if let Some(dy) = g.rise(block.min(crate::gfx::RISE_LAST)) {
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
    g.canvas.px = px;
}

/// Pictures taken for transitions so far: each taking gets its own number.
static PICTURES_MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Draw a full-screen picture scaled into `window`, with rounded corners, a
/// soft shadow that lifts it off what is underneath, and a hairline rim.
///
/// This runs every frame an app opens or closes, so it is written to cost
/// little: sampling is fixed-point, and the distance to the window's edge is
/// only worked out for pixels near that edge.
/// What a growing window shows besides its screen: the tile it grows out of,
/// read from the picture beneath and scaled with the window, and how much of
/// the window the screen has taken over (1: all screen, 0: all tile). Small,
/// the window is the tile; it becomes the screen as it grows, and the screen
/// melts back into the tile as it shrinks, so neither is swapped at an edge.
pub(crate) struct Morph<'a> {
    pub under: &'a [u32],
    pub tile: Rect,
    pub screen: f32,
}

impl Morph<'_> {
    /// How much of the window is the screen, as it grows from the tile
    /// (`grown` 0) to the panel (1): the tile alone at first, the screen
    /// alone from about two thirds of the way.
    pub fn screen_at(grown: f32) -> f32 {
        let t = ((grown - 0.08) / (0.62 - 0.08)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

pub(crate) fn blit_scaled(canvas: &mut Canvas, src: &[u32], window: Rect, radius: f32, edge: Color, morph: Option<&Morph>) {
    let (w, h) = (canvas.w, canvas.h);
    if window.w < 1.0 || window.h < 1.0 {
        return;
    }
    // Only a tile that is really there, and partly showing, is drawn.
    let morph = morph.filter(|m| m.screen < 1.0 && m.tile.w >= 1.0 && m.tile.h >= 1.0 && m.under.len() == src.len());
    // The whole panel: a straight copy.
    if morph.is_none() && window.w >= w as f32 - 0.5 && window.h >= h as f32 - 0.5 {
        canvas.px.copy_from_slice(src);
        return;
    }
    // Well inside the window, where no edge needs working out.
    let core = radius + 1.5;
    let inside = |x: f32, y: f32| {
        (x > window.x + 1.5 && x < window.right() - 1.5 && y > window.y + core && y < window.bottom() - core)
            || (x > window.x + core && x < window.right() - core && y > window.y + 1.5 && y < window.bottom() - 1.5)
    };

    // The shadow: a little lower than the window, as light from above casts it.
    const REACH: f32 = 8.0;
    let lifted = window.offset(0.0, 2.0);
    for y in ((window.y - REACH).floor() as i32).max(0)..((window.bottom() + REACH + 2.0).ceil() as i32).min(h) {
        for x in ((window.x - REACH).floor() as i32).max(0)..((window.right() + REACH).ceil() as i32).min(w) {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if inside(fx, fy) {
                continue;
            }
            let d = sd_rrect(fx, fy, &lifted, radius);
            if d > 0.0 && d < REACH {
                let fall = 1.0 - d / REACH;
                let at = (y * w + x) as usize;
                canvas.px[at] = blend(canvas.px[at], Color::BLACK, 0.22 * fall * fall);
            }
        }
    }

    let x0 = (window.x.floor() as i32).max(0);
    let y0 = (window.y.floor() as i32).max(0);
    let x1 = (window.right().ceil() as i32).min(w);
    let y1 = (window.bottom().ceil() as i32).min(h);
    // Where each column of the window reads from, worked out once: the two
    // source columns and how far between them, in 256ths.
    let (sx, sy) = (w as f32 / window.w, h as f32 / window.h);
    let columns: Vec<(usize, usize, u32)> = (x0..x1)
        .map(|x| {
            let u = ((x as f32 + 0.5 - window.x) * sx - 0.5).clamp(0.0, (w - 1) as f32);
            let u0 = u as usize;
            (u0, (u0 + 1).min(w as usize - 1), (u.fract() * 256.0) as u32)
        })
        .collect();
    // The tile, likewise: where each column of the window reads from in it.
    let tile_columns: Vec<(usize, usize, u32)> = match morph {
        Some(m) => (x0..x1)
            .map(|x| {
                let u = (m.tile.x + (x as f32 + 0.5 - window.x) * m.tile.w / window.w - 0.5).clamp(0.0, (w - 1) as f32);
                let u0 = u as usize;
                (u0, (u0 + 1).min(w as usize - 1), (u.fract() * 256.0) as u32)
            })
            .collect(),
        None => Vec::new(),
    };
    let screen = morph.map_or(256, |m| (m.screen.clamp(0.0, 1.0) * 256.0) as u32);
    let edge_px = edge.to_u32();
    let mut scaled = vec![0u32; columns.len()];
    let mut tile_row = vec![0u32; tile_columns.len()];
    for y in y0..y1 {
        let v = ((y as f32 + 0.5 - window.y) * sy - 0.5).clamp(0.0, (h - 1) as f32);
        let v0 = v as usize;
        let (top, bottom) = (v0 * w as usize, (v0 + 1).min(h as usize - 1) * w as usize);
        let fv = (v.fract() * 256.0) as u32;
        let fy = y as f32 + 0.5;
        let row = (y * w) as usize;
        // Bilinear, so shrunken text stays text rather than breaking up: the
        // whole row at once, four pixels at a time with NEON.
        let width = w as usize;
        bezel::simd::bilinear_row(&src[top..top + width], &src[bottom..bottom + width], &columns, fv, &mut scaled);
        if let Some(m) = morph {
            let v = (m.tile.y + (y as f32 + 0.5 - window.y) * m.tile.h / window.h - 0.5).clamp(0.0, (h - 1) as f32);
            let v0 = v as usize;
            let (top, bottom) = (v0 * width, (v0 + 1).min(h as usize - 1) * width);
            bezel::simd::bilinear_row(&m.under[top..top + width], &m.under[bottom..bottom + width], &tile_columns, (v.fract() * 256.0) as u32, &mut tile_row);
            for (px, &tile) in scaled.iter_mut().zip(&tile_row) {
                *px = mix_px(tile, *px, screen);
            }
        }
        for ((x, _), &px) in (x0..x1).zip(&columns).zip(&scaled) {
            let at = row + x as usize;
            let fx = x as f32 + 0.5;
            if inside(fx, fy) {
                canvas.px[at] = px;
                continue;
            }
            let d = sd_rrect(fx, fy, &window, radius);
            if d > 0.5 {
                continue;
            }
            // A one-pixel rim, so the window's edge reads against any ground.
            let rim = if d > -1.0 && radius > 0.5 { (90.0 * (d + 1.0).clamp(0.0, 1.0)) as u32 } else { 0 };
            let px = mix_px(px, edge_px, rim);
            let cover = ((0.5 - d).clamp(0.0, 1.0) * 256.0) as u32;
            canvas.px[at] = mix_px(canvas.px[at], px, cover);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    /// Keep everything the tests save (settings, keys, the robot's address,
    /// logs) in a scratch folder, never the computer's own `/etc` or `/data`.
    pub fn sandbox() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let dir = std::env::temp_dir().join(format!("catalyst-tests-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("CATALYST_CONF", dir.join("robot"));
            std::env::set_var("CATALYST_LOGS", dir.join("logs"));
        });
    }

    /// A shell on the demo robot, drawn once so Home knows where its tiles are.
    fn shell() -> Shell {
        sandbox();
        let mut shell = Shell::new(Robot::demo());
        shell.apply_settings();
        for _ in 0..120 {
            shell.tick(FRAME);
            shell.draw();
        }
        shell
    }

    fn frames(shell: &mut Shell, n: usize, dt: f32) {
        for _ in 0..n {
            shell.tick(dt);
            shell.draw();
        }
    }

    /// Enter on Home's focused tile: it is pressed in for an instant, then
    /// the app grows out of it. Returns once it has begun to.
    fn open(shell: &mut Shell, dt: f32) {
        shell.key(Key::Enter);
        for _ in 0..20 {
            if shell.transition.is_some() || shell.stack.len() > 1 {
                return;
            }
            frames(shell, 1, dt);
        }
        panic!("the app never opened");
    }

    #[test]
    fn a_tile_is_pressed_in_before_its_app_grows_out_of_it() {
        let mut shell = shell();
        shell.key(Key::Enter);
        frames(&mut shell, 2, FRAME);
        assert_eq!(shell.stack.len(), 1, "pressed in first");
        assert!(shell.gfx.press() > 0.5, "and seen pressed: {}", shell.gfx.press());
        frames(&mut shell, 5, FRAME);
        assert_eq!(shell.stack.len(), 2, "then the app grows out of it");
        // Changing one's mind while it is pressed in: nothing opens.
        let mut shell = self::shell();
        shell.key(Key::Enter);
        frames(&mut shell, 1, FRAME);
        shell.key(Key::Right);
        frames(&mut shell, 20, FRAME);
        assert_eq!(shell.stack.len(), 1);
    }

    #[test]
    fn a_key_after_an_idle_spell_starts_its_motion_from_the_beginning() {
        let mut shell = shell();
        // The device sleeps up to a quarter of a second between wake-ups.
        shell.tick(0.25);
        shell.key(Key::Enter);
        shell.tick(0.25);
        assert!(shell.gfx.press() > 0.99, "the press starts where it starts, not a wake-up in");
        assert_eq!(shell.progress(), None, "the tile is pressed in first");
        let mut n = 0;
        while shell.progress().is_none() {
            shell.tick(FRAME);
            n += 1;
            assert!(n < 10, "the app opens a moment later");
        }
        assert_eq!(shell.progress(), Some(0.0), "its first frame shows the start");
        shell.tick(FRAME);
        let p = shell.progress().unwrap();
        assert!(p > 0.0 && p < 0.1, "then one frame's worth of motion: {p}");
    }

    #[test]
    fn opening_can_be_turned_round_mid_flight_without_a_jump() {
        let mut shell = shell();
        open(&mut shell, FRAME);
        frames(&mut shell, 9, FRAME);
        let before = shell.progress().unwrap();
        assert!(before > 0.2 && before < 0.9, "{before}");
        shell.key(Key::Back);
        assert_eq!(shell.progress(), Some(before), "it turns round where it is");
        shell.tick(FRAME);
        // It was moving outward, so it carries on a little before coming back.
        let just_after = shell.progress().unwrap();
        assert!((just_after - before).abs() < 0.12, "no jump: {before} then {just_after}");
        frames(&mut shell, 120, FRAME);
        assert_eq!(shell.progress(), None);
        assert_eq!(shell.stack.len(), 1, "back on Home");
    }

    #[test]
    fn a_closing_app_can_be_asked_back_from_where_it_is() {
        let mut shell = shell();
        open(&mut shell, FRAME);
        frames(&mut shell, 120, FRAME);
        assert_eq!((shell.stack.len(), shell.progress()), (2, None));
        shell.key(Key::Back);
        frames(&mut shell, 8, FRAME);
        let closing = shell.progress().unwrap();
        assert!(closing > 0.1 && closing < 0.95, "{closing}");
        // Enter on the same tile: the same window returns, not a new one from the tile.
        shell.key(Key::Enter);
        assert_eq!(shell.stack.len(), 2);
        assert_eq!(shell.progress(), Some(closing));
        frames(&mut shell, 120, FRAME);
        assert_eq!((shell.stack.len(), shell.progress()), (2, None));
    }

    #[test]
    fn the_clock_and_the_greeting_read_as_they_are_set() {
        use chrono::TimeZone;
        let at = chrono::Local.with_ymd_and_hms(2026, 11, 18, 21, 5, 9).unwrap();
        let mut s = Settings::default();
        assert_eq!(s.clock(at), "21:05");
        s.seconds = true;
        assert_eq!(s.clock(at), "21:05:09");
        s.clock24 = false;
        assert_eq!(s.clock(at), "9:05:09");
        s.seconds = false;
        assert_eq!(s.clock(at), "9:05");
        assert_eq!(greeting(8, Some(5805)), "good morning · team 5805");
        assert_eq!(greeting(13, None), "good afternoon");
        assert_eq!(greeting(19, None), "good evening");
        assert_eq!(greeting(2, None), "hello");
    }

    #[test]
    fn the_wallpaper_holds_still_while_a_screen_comes_or_goes() {
        let mut shell = shell();
        // A move sets the wallpaper drifting; Enter at once opens an app.
        shell.key(Key::Right);
        frames(&mut shell, 2, FRAME);
        shell.key(Key::Left);
        frames(&mut shell, 1, FRAME);
        open(&mut shell, FRAME);
        frames(&mut shell, 1, FRAME);
        assert!(shell.transition.is_some());
        let window = shell.background_for;
        let mut seen = 0;
        while shell.transition.is_some() {
            frames(&mut shell, 1, FRAME);
            assert_eq!(shell.background_for, window, "the pictures stay good: the wallpaper under them has not moved");
            seen += 1;
            assert!(seen < 200, "the app finishes opening");
        }
        // Afterwards the drift carries on and settles, and the frames stop.
        frames(&mut shell, 240, FRAME);
        assert!(!shell.moving(), "everything comes to rest");
    }

    #[test]
    fn motion_does_not_depend_on_the_frame_rate() {
        // 0.3 s into opening an app at 60 frames a second and at 20.
        let at = |dt: f32| {
            let mut shell = shell();
            open(&mut shell, dt); // as far as the starting frame
            frames(&mut shell, (0.3 / dt).round() as usize, dt);
            shell.progress().unwrap()
        };
        let (smooth, slow) = (at(1.0 / 60.0), at(1.0 / 20.0));
        assert!((smooth - slow).abs() < 0.005, "60 fps {smooth} against 20 fps {slow}");
    }

    #[test]
    fn the_control_centre_comes_down_over_any_screen_and_takes_the_keys() {
        let mut shell = shell();
        // Over the Terminal, which otherwise takes every key for its program.
        let terminal = crate::screens::builtin_apps().position(|(_, _, name)| name == "Terminal").unwrap();
        for _ in 0..terminal {
            shell.key(Key::Right);
        }
        shell.key(Key::Enter);
        frames(&mut shell, 120, FRAME);
        assert_eq!(shell.stack.last().unwrap().0.title(), "Terminal");
        shell.key(Key::Ctrl(' '));
        assert!(shell.control_open());
        frames(&mut shell, 6, FRAME);
        let part_way = shell.control.shown.value();
        assert!(part_way > 0.0 && part_way < 1.0, "it comes down on a spring: {part_way}");
        assert!(shell.control_under.is_some(), "the screen behind is held still");
        // Keys go to the sheet: down to Light, and Enter flips it.
        let light = shell.settings.light;
        shell.key(Key::Down);
        shell.key(Key::Enter);
        assert_eq!(shell.settings.light, !light);
        assert_eq!(shell.stack.len(), 2, "nothing reached the screen behind");
        // Esc puts it away from wherever it is, and the terminal is still there.
        shell.key(Key::Back);
        assert!(!shell.control_open());
        frames(&mut shell, 120, FRAME);
        assert!(!shell.control.visible() && shell.control_under.is_none());
        assert_eq!(shell.stack.last().unwrap().0.title(), "Terminal");
    }

    #[test]
    fn up_from_the_top_of_home_pulls_the_control_centre_down() {
        let mut shell = shell();
        shell.key(Key::Up);
        assert!(shell.control_open());
        // Up again, at the sheet's top row, pushes it back.
        shell.key(Key::Up);
        assert!(!shell.control_open());
        // From the second row of tiles, Up only moves the focus.
        frames(&mut shell, 120, FRAME);
        shell.key(Key::Down);
        shell.key(Key::Up);
        assert!(!shell.control_open());
    }

    #[test]
    fn the_glance_page_is_one_step_left_of_home() {
        let mut shell = shell();
        let title = |shell: &Shell| shell.stack.last().unwrap().0.title().to_owned();
        assert_eq!(title(&shell), "Catalyst");
        shell.key(Key::Left);
        assert_eq!(title(&shell), "Glance");
        // Turned back while it is still sliding in.
        frames(&mut shell, 5, FRAME);
        shell.key(Key::Right);
        assert_eq!(title(&shell), "Catalyst");
        frames(&mut shell, 120, FRAME);
        // From the glance page, Enter opens the robot out of its card.
        shell.key(Key::Left);
        frames(&mut shell, 120, FRAME);
        shell.key(Key::Enter);
        let Some(Transition { path: Path::Grow(from), .. }) = &shell.transition else { panic!("the robot opens out of its card") };
        assert!(from.x > W * 0.5, "the card on the right: {from:?}");
        assert_eq!(shell.stack.len(), 2);
    }

    #[test]
    fn the_carousel_turns_one_app_at_a_time_and_opens_from_the_middle() {
        let mut shell = shell();
        shell.settings.home = HomeStyle::Carousel;
        shell.apply_settings();
        frames(&mut shell, 120, FRAME);
        shell.key(Key::Right);
        frames(&mut shell, 120, FRAME);
        open(&mut shell, FRAME);
        let Some(Transition { path: Path::Grow(from), .. }) = &shell.transition else { panic!("the app opens out of a card") };
        assert!((from.cx() - W * 0.5).abs() < 1.0, "the middle card: {from:?}");
        assert_eq!(shell.stack.last().unwrap().0.title(), "Health");
    }

    /// A shell with no robot yet, settled on Home.
    fn waiting() -> Shell {
        sandbox();
        let mut shell = Shell::new(Robot::none());
        shell.apply_settings();
        frames(&mut shell, 120, FRAME);
        shell
    }

    fn title(shell: &Shell) -> String {
        shell.stack.last().unwrap().0.title().to_owned()
    }

    fn says(shell: &Shell) -> String {
        shell.message.as_ref().map(|(text, _)| text.clone()).unwrap_or_default()
    }

    #[test]
    fn a_robot_linking_opens_the_robot_app_out_of_its_tile() {
        let mut shell = waiting();
        shell.robot = Robot::demo();
        shell.tick(FRAME);
        assert_eq!(title(&shell), "Robot");
        let Some(Transition { path: Path::Grow(from), .. }) = &shell.transition else { panic!("it grows out of the tile") };
        assert!(from.x < W * 0.4 && from.y < H * 0.5, "Robot's tile, top left: {from:?}");
        assert_eq!(shell.progress(), Some(0.0), "from its first frame");
        frames(&mut shell, 120, FRAME);
        // Esc goes back Home, and the robot still being there does not reopen it.
        shell.key(Key::Back);
        frames(&mut shell, 240, FRAME);
        assert_eq!((shell.stack.len(), title(&shell)), (1, "Catalyst".to_owned()));
    }

    #[test]
    fn home_turns_to_the_robot_tile_before_opening_it() {
        let mut shell = waiting();
        // On the second page of tiles.
        for _ in 0..3 {
            shell.key(Key::Right);
        }
        frames(&mut shell, 120, FRAME);
        shell.robot = Robot::demo();
        frames(&mut shell, 3, FRAME);
        assert_eq!(shell.stack.len(), 1, "the page turns back first");
        frames(&mut shell, 90, FRAME);
        assert_eq!(title(&shell), "Robot");
        let Some((_, Path::Grow(from))) = shell.stack.last() else { panic!("out of a tile") };
        assert!(from.x >= 0.0 && from.x < W * 0.4, "Robot's tile, back on screen: {from:?}");
        // The same from the carousel, turned away from Robot, and from the glance page.
        let mut shell = waiting();
        shell.settings.home = HomeStyle::Carousel;
        shell.apply_settings();
        shell.key(Key::Right);
        shell.key(Key::Right);
        frames(&mut shell, 120, FRAME);
        shell.robot = Robot::demo();
        frames(&mut shell, 120, FRAME);
        let Some((screen, Path::Grow(from))) = shell.stack.last() else { panic!("out of the middle card") };
        assert_eq!(screen.title(), "Robot");
        assert!((from.cx() - W * 0.5).abs() < 1.0, "{from:?}");
        let mut shell = waiting();
        shell.key(Key::Left);
        frames(&mut shell, 120, FRAME);
        shell.robot = Robot::demo();
        frames(&mut shell, 2, FRAME);
        let Some((screen, Path::Grow(from))) = shell.stack.last() else { panic!("out of the robot's card") };
        assert_eq!(screen.title(), "Robot");
        assert!(from.x > W * 0.5, "{from:?}");
    }

    #[test]
    fn a_robot_linking_elsewhere_is_only_announced() {
        // In another app: it stays in front, and the island says so.
        let mut shell = waiting();
        shell.key(Key::Right);
        shell.key(Key::Enter);
        frames(&mut shell, 200, FRAME);
        assert_eq!(title(&shell), "Health");
        shell.robot = Robot::demo();
        frames(&mut shell, 30, FRAME);
        assert_eq!((shell.stack.len(), title(&shell)), (2, "Health".to_owned()));
        assert!(says(&shell).ends_with("linked"), "{:?}", says(&shell));
        // Under the control centre, and with the setting off: the same.
        let mut shell = waiting();
        shell.key(Key::Ctrl(' '));
        frames(&mut shell, 60, FRAME);
        shell.robot = Robot::demo();
        frames(&mut shell, 30, FRAME);
        assert_eq!(shell.stack.len(), 1);
        let mut shell = waiting();
        shell.settings.open_robot = false;
        shell.robot = Robot::demo();
        frames(&mut shell, 30, FRAME);
        assert_eq!(shell.stack.len(), 1);
        assert!(says(&shell).ends_with("linked"));
        // A key pressed while Home is still turning to the tile keeps the screen.
        let mut shell = waiting();
        for _ in 0..3 {
            shell.key(Key::Right);
        }
        frames(&mut shell, 120, FRAME);
        shell.robot = Robot::demo();
        frames(&mut shell, 3, FRAME);
        shell.key(Key::Right);
        frames(&mut shell, 120, FRAME);
        assert_eq!(shell.stack.len(), 1);
    }

    #[test]
    fn a_link_that_blinks_is_not_a_robot_arriving() {
        let mut shell = waiting();
        shell.robot = Robot::demo();
        frames(&mut shell, 120, FRAME);
        shell.key(Key::Back);
        frames(&mut shell, 120, FRAME);
        assert_eq!(shell.stack.len(), 1);
        // Gone for a second and back: Home is left alone.
        shell.robot = Robot::none();
        frames(&mut shell, 60, FRAME);
        shell.robot = Robot::demo();
        frames(&mut shell, 60, FRAME);
        assert_eq!(shell.stack.len(), 1);
        // Gone for five (the robot restarted, or another was plugged in): it opens again.
        shell.robot = Robot::none();
        frames(&mut shell, 300, FRAME);
        shell.robot = Robot::demo();
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Robot");
    }

    #[test]
    fn back_then_quickly_another_page_opens_that_page() {
        let mut shell = shell();
        shell.open(Box::new(crate::screens::SettingsApp::new()), Path::Slide);
        frames(&mut shell, 120, FRAME);
        shell.key(Key::Down);
        shell.key(Key::Enter);
        frames(&mut shell, 120, FRAME);
        assert_eq!(title(&shell), "Appearance");
        // Back, and before Appearance has gone, Down and Enter: Text, not Appearance again.
        shell.key(Key::Back);
        frames(&mut shell, 3, FRAME);
        shell.key(Key::Down);
        shell.key(Key::Enter);
        assert_eq!(title(&shell), "Text");
        frames(&mut shell, 120, FRAME);
        // While the same page is still asked back from where it is.
        shell.key(Key::Back);
        frames(&mut shell, 3, FRAME);
        let leaving = shell.progress().unwrap();
        shell.key(Key::Enter);
        assert_eq!((title(&shell), shell.progress()), ("Text".to_owned(), Some(leaving)));
    }

    #[test]
    fn a_key_while_the_control_centre_goes_lands_on_a_live_screen() {
        let mut shell = shell();
        shell.key(Key::Ctrl(' '));
        frames(&mut shell, 120, FRAME);
        shell.key(Key::Back);
        frames(&mut shell, 2, FRAME);
        assert!(shell.control.visible(), "still on its way out");
        // Enter opens the focused app, and it is seen opening, not hidden behind a still picture.
        open(&mut shell, FRAME);
        assert!(!shell.control.visible() && shell.control_under.is_none());
        assert_eq!((shell.stack.len(), title(&shell)), (2, "Robot".to_owned()));
    }

    #[test]
    fn a_robot_back_from_a_match_shows_how_it_went() {
        // Plugged in at Home: the Robot page opens, then the match's summary
        // once its log has been pulled (four seconds, for the demo robot).
        let mut shell = waiting();
        shell.robot = Robot::demo_back_from_a_match();
        frames(&mut shell, 120, FRAME);
        assert_eq!(title(&shell), "Robot");
        frames(&mut shell, 60 * 4, FRAME);
        assert_eq!((shell.stack.len(), title(&shell)), (3, "Match".to_owned()));
        assert!(says(&shell).contains("q14"), "{:?}", says(&shell));
        // Back returns to the Robot page.
        shell.key(Key::Back);
        frames(&mut shell, 120, FRAME);
        assert_eq!(title(&shell), "Robot");

        // Busy in the Robot page meanwhile: only the island says so.
        let mut shell = waiting();
        shell.robot = Robot::demo_back_from_a_match();
        frames(&mut shell, 120, FRAME);
        shell.key(Key::Down);
        // The log arrives four seconds after the link; the island says so for a while.
        frames(&mut shell, 60 * 3, FRAME);
        assert_eq!(title(&shell), "Robot");
        assert!(says(&shell).ends_with("summary is in Logs"), "{:?}", says(&shell));

        // And with the setting off, nothing at all.
        let mut shell = waiting();
        shell.settings.match_report = false;
        shell.robot = Robot::demo_back_from_a_match();
        frames(&mut shell, 60 * 6, FRAME);
        assert_eq!(title(&shell), "Robot");
    }

    #[test]
    fn setup_in_the_simulator_saves_the_team_but_keeps_the_demo_robot() {
        let mut shell = shell();
        assert!(shell.robot.is_demo());
        shell.start_setup();
        frames(&mut shell, 30, FRAME);
        shell.key(Key::Enter);
        for c in "5805".chars() {
            shell.key(Key::Char(c));
        }
        shell.key(Key::Enter);
        assert_eq!(std::fs::read_to_string(crate::config_path()).unwrap(), "5805", "the team is saved");
        assert!(shell.robot.is_demo(), "and the demo robot still stands in for it");
    }

    #[test]
    fn first_start_sets_the_handheld_up_and_gives_way_to_home() {
        let mut shell = shell();
        // A handheld out of the box: no robot yet.
        shell.robot = Robot::none();
        shell.start_setup();
        frames(&mut shell, 30, FRAME);
        assert_eq!(title(&shell), "Setup");
        let light = shell.settings.light;
        let accent = shell.settings.accent;
        // Set up here; team 5805; no Wi-Fi now; keys skipped; choices: the first off.
        shell.key(Key::Enter);
        for c in "5805".chars() {
            shell.key(Key::Char(c));
        }
        shell.key(Key::Enter);
        shell.key(Key::Down);
        shell.key(Key::Enter);
        shell.key(Key::Tab);
        assert!(shell.settings.open_robot);
        shell.key(Key::Enter);
        assert!(!shell.settings.open_robot, "the first choice switched off");
        shell.key(Key::Tab);
        // The look: the next accent, light mode, then Finish.
        shell.key(Key::Right);
        shell.key(Key::Down);
        shell.key(Key::Enter);
        assert_ne!(shell.settings.accent, accent);
        assert_eq!(shell.settings.light, !light);
        for _ in 0..3 {
            shell.key(Key::Down);
        }
        shell.key(Key::Enter);
        // "Just a moment", "all set", then Home, fading in under it.
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Setup");
        frames(&mut shell, 150, FRAME);
        assert_eq!((shell.stack.len(), title(&shell)), (1, "Catalyst".to_owned()));
        assert!(crate::setup_done(), "not shown again");
        assert_eq!(std::fs::read_to_string(crate::config_path()).unwrap(), "5805");
        assert_eq!(shell.robot.address, "10.58.5.2");
        frames(&mut shell, 60, FRAME);
        assert!(shell.curtain.is_none(), "the fade has finished");
    }

    #[test]
    fn the_glance_page_runs_left_in_pages_of_two_cards() {
        let mut shell = shell();
        // Device, robot | next match, last match.
        shell.key(Key::Left);
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Glance");
        // On the robot's card first, the nearer one; left to the device's.
        shell.key(Key::Left);
        // Left again: the next page, on its right-hand card (the last match).
        shell.key(Key::Left);
        frames(&mut shell, 60, FRAME);
        shell.key(Key::Enter);
        frames(&mut shell, 120, FRAME);
        assert_eq!(title(&shell), "Match", "the last match's card opens its summary");
        shell.key(Key::Back);
        frames(&mut shell, 120, FRAME);
        // Its neighbour is the next match: the schedule.
        shell.key(Key::Left);
        shell.key(Key::Enter);
        frames(&mut shell, 120, FRAME);
        assert_eq!(title(&shell), "Schedule");
        shell.key(Key::Back);
        frames(&mut shell, 120, FRAME);
        // Right four times: across this page's two cards, the first page's two, then Home.
        for _ in 0..4 {
            shell.key(Key::Right);
        }
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Catalyst");
        // A pinned value, chosen in Tables, has a card of its own.
        shell.settings.pinned = Some("Loop/Robot/LastMs".into());
        shell.settings.glance = vec![Widget::Pinned];
        shell.apply_settings();
        shell.key(Key::Left);
        frames(&mut shell, 60, FRAME);
        shell.key(Key::Enter);
        frames(&mut shell, 120, FRAME);
        assert_eq!(title(&shell), "Robot", "the pinned value opens in its table");
    }

    #[test]
    fn the_start_gives_way_to_home_by_itself_or_at_a_key() {
        let mut shell = shell();
        shell.start_intro("test");
        frames(&mut shell, 30, FRAME);
        assert!(shell.intro.is_some());
        // A key skips it, and is spent doing so: Home's focus has not moved.
        shell.key(Key::Right);
        assert!(shell.intro.as_ref().unwrap().leaving());
        frames(&mut shell, 90, FRAME);
        assert!(shell.intro.is_none(), "gone once it has faded");
        shell.key(Key::Enter);
        frames(&mut shell, 120, FRAME);
        assert_eq!(title(&shell), "Robot", "the first tile was still the one focused");
        // Left alone, it plays through and leaves on its own.
        let mut alone = super::tests::shell();
        alone.start_intro("test");
        frames(&mut alone, ((crate::intro::DONE + 1.0) / FRAME) as usize, FRAME);
        assert!(alone.intro.is_none());
    }

    /// Random keys at random frame times, with robots coming and going and
    /// settings changing under it, on every screen: nothing may panic.
    /// Off the handheld only, where the terminal is a stand-in and Files
    /// keeps to a scratch folder: on Linux the keys would reach a real shell.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn random_keys_never_break_the_shell() {
        let scratch = std::env::temp_dir().join(format!("catalyst-monkey-{}", std::process::id()));
        std::fs::create_dir_all(scratch.join("files/sub")).unwrap();
        std::fs::write(scratch.join("files/notes.txt"), "a line\n\tindented\nhéllo wörld ✓\n").unwrap();
        std::fs::write(scratch.join("files/sub/empty.txt"), "").unwrap();
        // Read once by Files and by the editor.
        std::env::set_var("CATALYST_FILES", scratch.join("files"));
        let keys = [
            Key::Up, Key::Down, Key::Left, Key::Right, Key::Enter, Key::Back, Key::Backspace, Key::Tab, Key::PageUp, Key::PageDown,
            Key::Char('n'), Key::Char('d'), Key::Char('x'), Key::Char('é'), Key::Char(' '), Key::Char('a'), Key::Char('✓'),
            Key::Ctrl('q'), Key::Ctrl(' '), Key::Ctrl('s'), Key::Ctrl('k'), Key::Leave,
        ];
        // A small, fixed generator: the same run every time, so a failure can be replayed.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        sandbox();
        for run in 0..12 {
            let mut shell = Shell::new(if run % 2 == 0 { Robot::demo() } else { Robot::none() });
            shell.apply_settings();
            for step in 0..2500 {
                match next(100) {
                    0 => shell.robot = if shell.robot.linked() { Robot::none() } else { Robot::demo() },
                    1 => {
                        shell.settings.home = shell.settings.home.next();
                        shell.apply_settings();
                    }
                    2 => {
                        shell.settings.text = shell.settings.text.next();
                        shell.apply_settings();
                    }
                    3 => {
                        shell.settings.calm = !shell.settings.calm;
                        shell.apply_settings();
                    }
                    _ => {}
                }
                // Several keys in one frame now and then, as a fast typist sends them.
                for _ in 0..1 + next(3) / 2 {
                    shell.key(keys[next(keys.len())]);
                }
                // Mostly a frame beat; sometimes a long gap, sometimes none.
                let dt = [0.0, 0.016, 0.025, 0.025, 0.05, 0.1, 0.25][next(7)];
                if shell.tick(dt) || step % 7 == 0 {
                    shell.draw();
                }
            }
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn the_calm_version_changes_screens_at_once() {
        let mut shell = shell();
        shell.settings.calm = true;
        shell.key(Key::Enter);
        assert_eq!((shell.stack.len(), shell.progress()), (2, None));
        shell.key(Key::Back);
        assert_eq!((shell.stack.len(), shell.progress()), (1, None));
    }

    #[test]
    fn the_tour_follows_first_boot_setup_once() {
        let mut shell = shell();
        shell.start_setup();
        frames(&mut shell, 30, FRAME);
        // Setup put off from its first page ("later"): Home, then the tour.
        shell.key(Key::Down);
        shell.key(Key::Down);
        shell.key(Key::Enter);
        frames(&mut shell, 60 * 3, FRAME);
        assert_eq!((shell.stack.len(), shell.tour_step()), (1, Some(0)), "offered once Home has arrived");
        // It takes the keys: Enter goes on, and Home's focus does not move.
        shell.key(Key::Enter);
        shell.key(Key::Right);
        assert_eq!(shell.tour_step(), Some(2));
        shell.key(Key::Back);
        frames(&mut shell, 60, FRAME);
        assert_eq!(shell.tour_step(), None, "skipped, it fades away");
        assert!(shell.settings.toured, "and is not offered again");
    }

    #[test]
    fn settings_help_takes_the_tour_from_home() {
        let mut shell = shell();
        shell.settings.toured = true;
        // An app open over Home, and Settings › Help › Take the tour over that.
        open(&mut shell, FRAME);
        frames(&mut shell, 60, FRAME);
        let tour = crate::screens::setting_names().find(|(_, n)| *n == "Take the tour").map(|(i, _)| i).unwrap();
        shell.open(crate::screens::open_setting(tour), Path::Slide);
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Help");
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        assert_eq!((shell.stack.len(), shell.tour_step()), (1, Some(0)), "back on Home, touring");
    }

    #[test]
    fn question_mark_brings_up_the_shortcuts_except_where_it_types() {
        let mut shell = shell();
        frames(&mut shell, 30, FRAME);
        shell.key(Key::Char('?'));
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Shortcuts");
        shell.key(Key::Char('?'));
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Catalyst", "? again puts it away");
        // In search, ? is typed.
        shell.key(Key::Char(' '));
        frames(&mut shell, 60, FRAME);
        let searching = title(&shell);
        shell.key(Key::Char('?'));
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), searching);
    }

    #[test]
    fn developer_options_slow_motion_and_show_the_keys() {
        // 0.2 s into an app opening; at 5x the tile's press takes longer too.
        let at = |scale: f32| {
            let mut shell = shell();
            (shell.settings.developer, shell.settings.motion_scale) = (true, scale);
            shell.key(Key::Enter);
            while shell.transition.is_none() {
                frames(&mut shell, 1, FRAME);
            }
            frames(&mut shell, 12, FRAME);
            shell.progress().unwrap()
        };
        let (normal, slow) = (at(1.0), at(5.0));
        assert!(slow < normal * 0.6, "5x: {slow} against {normal}");
        // Without developer options, the scale is not used.
        let mut shell = shell();
        (shell.settings.developer, shell.settings.motion_scale, shell.settings.show_keys) = (false, 5.0, true);
        assert_eq!(shell.motion_scale(), 1.0);
        shell.key(Key::Down);
        assert!(shell.echo.is_empty(), "nor are keys shown");
        shell.settings.developer = true;
        shell.key(Key::Down);
        shell.key(Key::Enter);
        assert_eq!(shell.echo.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["↓", "enter"]);
        frames(&mut shell, 120, FRAME);
        assert!(shell.echo.is_empty(), "they fade after a moment");
    }
}
