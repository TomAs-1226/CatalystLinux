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
    /// The widgets on the glance page and among Home's apps (widgets.rs).
    pub widgets: crate::widgets::Layout,
    /// Settings › Home › Widgets asked for a board to be edited, back on
    /// Home. Never kept.
    pub edit_widgets: Option<crate::widgets::Board>,
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
    /// Standby: how long with no key before the screen goes off (seconds;
    /// 0 never), and how the processor chooses its speed.
    pub screen_off: u32,
    pub power: crate::standby::Power,
    /// Settings › Battery: the saver, low-battery care, Wi-Fi in standby,
    /// the red light.
    pub battery: crate::power::Choices,
    /// The backlight's level, in percent, kept across restarts: it starts
    /// dark and rises to this once the first picture is on the panel.
    pub brightness: u32,
    /// Where there is a motion sensor: the screen comes on when the
    /// handheld is picked up, and goes off when it is put face down.
    pub lift_wake: bool,
    pub face_down: bool,
    /// Settings › Lock screen and StandBy: the lock screen's look, and
    /// StandBy on the charger (the passcode itself is kept apart, passcode.rs).
    pub lock: crate::lock::Choices,
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
            widgets: crate::widgets::Layout::starting(),
            edit_widgets: None,
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
            screen_off: 60,
            power: crate::standby::Power::Balanced,
            battery: crate::power::Choices::default(),
            brightness: 70,
            lift_wake: true,
            face_down: true,
            lock: crate::lock::Choices::default(),
        }
    }
}

/// How Home lays the apps out.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HomeStyle {
    /// Tiles, six to a page.
    Grid,
    /// One large card at a time, turned through like a dial.
    Carousel,
    /// Album covers in a cover flow, on the GPU when it helps (flow.rs).
    Flow,
}

impl HomeStyle {
    pub const ALL: [HomeStyle; 3] = [HomeStyle::Grid, HomeStyle::Carousel, HomeStyle::Flow];

    pub fn name(self) -> &'static str {
        match self {
            HomeStyle::Grid => "grid",
            HomeStyle::Carousel => "carousel",
            HomeStyle::Flow => "flow",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.name() == name)
    }

    pub fn next(self) -> Self {
        self.step(1)
    }

    /// The style `step` along (−1 the one before), round and round.
    pub fn step(self, step: i32) -> Self {
        let at = Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as i32;
        Self::ALL[(at + step).rem_euclid(Self::ALL.len() as i32) as usize]
    }

    /// One app at a time in the middle (the carousel and the flow), not a grid.
    pub fn one_at_a_time(self) -> bool {
        self != HomeStyle::Grid
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

/// Set by a screen that changed `Cx::settings` in its `update` (a key's
/// changes are drawn anyway): the shell draws them at once.
static SETTINGS_CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The settings were changed outside a key: apply them.
pub fn settings_changed() {
    SETTINGS_CHANGED.store(true, std::sync::atomic::Ordering::Relaxed);
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
    /// Bring Spotlight up over this screen (Space on Home).
    Spotlight,
    /// Done: give way to the screen beneath, fading out over it as it appears.
    Done,
    /// Back to Home, and take the tour there.
    Tour,
    /// Back to Home from however deep (Settings › Home › Widgets, for Home
    /// to edit its widgets).
    Home,
}

pub trait Screen {
    fn title(&self) -> &str;
    /// The app it is of, as a notification names it: its title, unless the
    /// title changes with its page.
    fn app(&self) -> &str {
        self.title()
    }
    /// The live activity this page shows for itself under another title
    /// (Music's lyrics, titled by the song): a change to it does not open
    /// the island over the page. (By title, "Music" and "Assistant" are.)
    fn shows_activity(&self) -> Option<crate::liveactivity::Kind> {
        None
    }
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
    /// Something on the page that changes on its own and is not motion: the
    /// field's robot, as often as its pose arrives; a replay playing. Asked
    /// on every look with the time that has really passed (motion's stands
    /// still at rest), with the robot as just refreshed. True to be drawn.
    /// It never holds the frame beat: the main loop looks in as often as
    /// [`Screen::ambient`] says, as it does for Music's bars (0.8.1).
    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        false
    }
    /// How often the main loop looks in for [`Screen::ambient_tick`] while
    /// nothing moves; `None` for the usual four times a second.
    fn ambient(&self) -> Option<std::time::Duration> {
        None
    }
    /// A word for the island when the screen opens.
    fn greeting(&self) -> Option<&'static str> {
        None
    }
    /// The screen is on top again after one above it closed.
    fn resume(&mut self) {}
    /// The screen has been left (it may wait among the recent apps): what it
    /// was doing only for whoever looks at it stops (the on-device model's
    /// answer under way).
    fn set_aside(&mut self) {}
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
    /// An external app this screen wants the panel handed to. Asked whenever
    /// the screen has landed and shows alone; the shell closes the screen
    /// once the app ends.
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
    /// Kept among the recent apps once left. Not a screen that takes the
    /// whole panel (setup), nor a passing one (search, the shortcut list).
    fn keep_in_recents(&self) -> bool {
        !self.full_screen()
    }
    /// Something on it needs watching, so the screen does not dim or go off
    /// while it shows (a program running in a terminal, a match timer).
    fn keeps_awake(&self) -> bool {
        false
    }
    /// Enter held down repeats, as a remote's button does when held (the
    /// Remote app's buttons). Elsewhere Enter counts once a press.
    fn repeats_enter(&self) -> bool {
        false
    }
    /// A lock screen: nothing that reaches past it opens over it while it
    /// is anywhere on the stack (Spotlight; see `Shell::locked`).
    fn locks(&self) -> bool {
        false
    }
    /// What is typed here is secret (a passcode): Show key presses leaves
    /// its keys out.
    fn secret(&self) -> bool {
        false
    }
    /// What push-to-talk does here ([`crate::voice`]): a question for the
    /// assistant (the assistant itself), words typed in at the cursor (a
    /// note, a file), or nothing of its own (None: the assistant opens with
    /// what was said as its question).
    fn dictation(&self) -> Option<crate::voice::Dictation> {
        None
    }
    /// What was said, written down, for a screen that takes dictation.
    fn dictated(&mut self, _text: &str, _cx: &mut Cx) -> Nav {
        Nav::Stay
    }
    /// Copy (Ctrl-C; clipboard.rs): what this screen copies, where it
    /// knows better than the shell. Otherwise the shell copies the field
    /// being typed in, else the value of the row in focus, as last drawn.
    fn copy(&mut self, _robot: &Robot) -> Option<String> {
        None
    }
    /// Paste (Ctrl-V, or Enter in the clipboard's history): `text` put in
    /// where the typing is, on a screen that `wants_text`. By default it is
    /// typed in a character at a time on one line (a line break as a space),
    /// so every text field takes a paste with nothing of its own; one that
    /// takes lines (the editor, the terminal) puts them in as they are.
    fn paste(&mut self, text: &str, cx: &mut Cx) -> Nav {
        for c in crate::clipboard::one_line(text).chars() {
            match self.key(Key::Char(c), cx) {
                Nav::Stay => {}
                other => return other,
            }
        }
        Nav::Stay
    }
    /// A terminal: Ctrl-C and Ctrl-V are its programs' (the interrupt, and
    /// the next key as it is). Copy and paste there are Ctrl-Shift-C and V,
    /// which reach the screen as keys (`Key::Ctrl('C')`, `Key::Ctrl('V')`).
    fn keeps_ctrl(&self) -> bool {
        false
    }
}

const MESSAGE_SECONDS: f32 = 2.4;

/// The most stops the pager shows a dot each for; past that, a track
/// `SCRUB_W` wide with the place marked on it.
const PAGER_DOTS: usize = 23;
const SCRUB_W: f32 = 64.0;

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
pub(crate) enum Path {
    Slide,
    Grow(Rect),
}

impl Path {
    fn spring(&self) -> Spring {
        match self {
            // A whole page arriving, as long as Bezel's page turn but with a
            // settle that shows: it runs about 5 px past and comes back.
            Path::Slide => crate::gfx::spatial::slow(),
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
    /// What an arriving screen grows over when it is not the screen beneath
    /// it: the recent apps as they showed, for a card to grow out of, or
    /// Home, for an app brought back several pages deep.
    backdrop: Option<Vec<u32>>,
    /// A screen that arrives while another transition was still under way
    /// (a page pushed as the last slides in, an app opened as another goes
    /// back into its tile): that motion goes on playing beneath this one,
    /// from where it was and at its speed, rather than landing at once.
    beneath: Option<Box<Transition>>,
    /// A screen that leaves while the one above it was still leaving (Esc
    /// pressed again before a page had gone): that motion goes on playing
    /// within this one's picture.
    within: Option<Box<Transition>>,
    /// Pictures only: what it moves has already been let go (an app put
    /// among the recent apps, a page done with), or it plays on beneath or
    /// within another transition.
    ghost: bool,
    /// Room for what plays beneath and within, drawn as one picture each.
    scratch: [Vec<u32>; 2],
    /// Arriving: how deep the stack was before it, so the screens above that
    /// are the ones it brings in (an app brought back can bring several).
    over: usize,
    /// It grew over its backdrop (the recent apps as they showed), not the
    /// screen beneath: turned back, it lands on a picture that is not that
    /// screen, which then fades into it.
    backdropped: bool,
}

impl Transition {
    fn new(path: Path, from: f32, to: f32, leaving: Option<Box<dyn Screen>>, backdrop: Option<Vec<u32>>) -> Self {
        let mut progress = Animated::new(from, path.spring()).with_rest(0.002);
        progress.set(to);
        Transition { path, progress, pictures: None, made: 0, leaving, backdrop, beneath: None, within: None, ghost: false, scratch: Default::default(), over: 0, backdropped: false }
    }

    /// A screen on its way in (not one leaving, nor a ghost).
    fn arriving(&self) -> bool {
        self.leaving.is_none() && !self.ghost
    }

    /// Arriving and never yet drawn, from where it starts: nothing of what it
    /// brings has shown. (Closed and asked back before a frame, a screen is
    /// undrawn too, but it was showing whole all along.)
    fn unseen(&self) -> bool {
        self.arriving() && self.pictures.is_none() && self.progress.value() < 0.5 && self.within.is_none()
    }

    /// On its way out: a screen leaving, or the ghost of one.
    fn closing(&self) -> bool {
        self.progress.target() < 0.5
    }

    /// It moves over its backdrop (a picture of what showed), not over the
    /// screen beneath, or would once drawn.
    fn over_backdrop(&self) -> bool {
        self.backdropped || self.backdrop.is_some()
    }

    /// Advance what plays beneath and within, if anything; true while any
    /// of it moves. Each is let go once it has landed: the picture it then
    /// makes is the one it plays beneath or within.
    fn tick_nested(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for nested in [&mut self.beneath, &mut self.within] {
            // Both must be advanced, so no short-circuit.
            if nested.as_mut().is_some_and(|t| t.progress.tick(dt) | t.tick_nested(dt)) {
                moving = true;
            } else {
                *nested = None;
            }
        }
        moving
    }

    /// The two pictures it moves (what it was taken over, and the screen it
    /// brings or takes away), with what plays beneath or within it drawn
    /// into them as it now stands. None until its pictures are taken.
    fn layers(&mut self, background: &[u32], radius: f32, edge: Color) -> Option<(&[u32], &[u32])> {
        let Transition { pictures, beneath, within, scratch, .. } = self;
        let [(under, _), (over, _)] = pictures.as_ref()?;
        let [under_scratch, over_scratch] = scratch;
        let under = match beneath.as_mut().filter(|b| b.pictures.is_some()) {
            Some(b) => {
                b.compose(background, radius, edge, under_scratch);
                under_scratch.as_slice()
            }
            None => under.as_slice(),
        };
        let over = match within.as_mut().filter(|w| w.pictures.is_some()) {
            Some(w) => {
                w.compose(background, radius, edge, over_scratch);
                over_scratch.as_slice()
            }
            None => over.as_slice(),
        };
        Some((under, over))
    }

    /// Draw this transition as it now stands into `out`, on the processor
    /// (it plays beneath or within another). `radius` is a tile's corner.
    fn compose(&mut self, background: &[u32], radius: f32, edge: Color, out: &mut Vec<u32>) {
        let (path, p) = (self.path, self.progress.value());
        out.clear();
        let Some((under, over)) = self.layers(background, radius, edge) else {
            return out.extend_from_slice(background);
        };
        match path {
            Path::Slide => {
                out.extend_from_slice(background);
                compose_slide(out, under, over, p);
            }
            Path::Grow(from) => {
                // As the top transition draws it (`draw_frame`).
                let grown = p.clamp(0.0, 1.0);
                out.extend_from_slice(under);
                let window = from.lerp(&Rect::new(0.0, 0.0, W, H), grown);
                let morph = Morph { under, tile: from, screen: Morph::screen_at(grown) };
                scale_into(out, (W as i32, H as i32), over, window, radius * (1.0 - grown), edge, Some(&morph));
            }
        }
    }

    /// Its pictures just taken: a closing motion playing beneath or within
    /// it goes on over what it now shows there (Home as it is now, not as it
    /// was when that motion began), so it lands on exactly that picture.
    fn rebase(&mut self) {
        let Transition { pictures: Some([(under, _), (over, _)]), beneath, within, .. } = self else { return };
        for (nested, base) in [(beneath, &*under), (within, &*over)] {
            if let Some(Transition { pictures: Some([(was, _), _]), .. }) = nested.as_deref_mut().filter(|t| t.closing()) {
                was.clone_from(base);
            }
        }
    }

    /// The opening of the screen arriving by `path`, still playing somewhere
    /// beneath this one (a page was pushed as it grew): taken out, with
    /// whatever plays beneath it in turn, for that screen's closing to turn
    /// it round.
    fn take_opening(&mut self, path: Path) -> Option<Box<Transition>> {
        let below = self.beneath.as_mut()?;
        if below.path == path && !below.closing() {
            return self.beneath.take();
        }
        below.take_opening(path)
    }

    /// No more than `depth` motions played inside one another below this one
    /// (each is a picture drawn every frame): any deeper is drawn into the
    /// picture it plays in as it stands now, and holds still there, rather
    /// than vanishing.
    fn prune(&mut self, depth: usize, background: &[u32], radius: f32, edge: Color) {
        let Transition { pictures, beneath, within, .. } = self;
        for (nested, at) in [(beneath, 0), (within, 1)] {
            let Some(t) = nested else { continue };
            if depth > 0 {
                t.prune(depth - 1, background, radius, edge);
                continue;
            }
            if let Some(pictures) = pictures.as_mut() {
                let mut held = Vec::new();
                t.compose(background, radius, edge, &mut held);
                pictures[at].0 = held;
            }
            *nested = None;
        }
    }
}

/// A slide's frame: the page beneath drifting a third as far as the page on
/// top travels, at `p` (0 to 1, a little past either end as it settles).
fn compose_slide(dst: &mut [u32], under: &[u32], over: &[u32], p: f32) -> (f32, f32) {
    let (under_x, over_x) = ((-p * W * 0.3).round(), ((1.0 - p) * W).round());
    blit_shifted(dst, under, W as usize, H as usize, under_x as i32);
    blit_shifted(dst, over, W as usize, H as usize, over_x as i32);
    (under_x, over_x)
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
    /// Spotlight, over whatever screen is showing; while it is out, the frame
    /// it came up over (the screen and the status, not yet the island's
    /// glass, which it grows out of) and the page dots that frame asked for.
    spotlight: crate::spotlight::Spotlight,
    spotlight_under: Option<(Vec<u32>, Option<(usize, usize, f32)>)>,
    /// Spotlight's index has been set reading, ahead of its first search.
    spotlight_warmed: bool,
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
    /// The shell's own clock, in seconds, which `?` being held is timed by.
    clock: f32,
    /// `?` held on a screen not taking text: since when, when it last came
    /// (a held key keeps coming, repeating), the countdown last said, and
    /// whether the shortcut list has opened.
    help_hold: Option<HelpHold>,
    /// Apps left recently, kept alive as they were; the switcher while it
    /// shows, and the screen it came up over (held still behind it).
    recents: crate::recents::Recents,
    switcher: Option<crate::recents::Switcher>,
    switcher_under: Option<(Vec<u32>, Vec<u32>)>,
    /// A screenshot being saved, and the flash that says it was taken.
    saving: Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    flash: Option<Animated>,
    /// Push-to-talk (Alt held, anywhere) and answers said aloud.
    voice: crate::voice::Talk,
    /// The island's width at rest (no message), which the title keeps its
    /// room beside; and how much of the title shows, as an island grown
    /// wider than that passes over it.
    island_rest: f32,
    title_shown: Animated,
    /// The same for the status at the top right (links, battery, time),
    /// and where it begins, as last drawn.
    status_shown: Animated,
    status_left: f32,
    /// Standby: after a while with no key the screen dims, then goes off.
    standby: crate::standby::Standby,
    /// The battery: its readings, the saver's rules, low-battery care, and
    /// what deep standby switches off.
    care: crate::power::Care,
    /// The keyboard's lights, borrowed for notices and warnings.
    lights: crate::lights::Lights,
    /// Deep standby: after a long while with the screen off, everything a
    /// key need not have, off.
    deep: crate::deep::Deep,
    /// The key that woke it from deep standby, and when it last came (on
    /// the shell's clock): held, its repeats are spent too.
    woke_by: Option<(Key, f32)>,
    /// Matches already warned about for queueing: (match, 0 soon / 1 now).
    queue_warned: Vec<(String, u8)>,
    queue_age: f32,
    /// When the queue alert would next speak, worked out ahead: deep
    /// standby's minute tick looks only at this (the schedule is parked).
    queue_due: Option<i64>,
    /// Reminders: when the next is due, worked out ahead the same way.
    reminders: crate::reminders::Watch,
    /// Deep standby woke for one of those: nobody was looking.
    woke_for_alert: bool,
    /// StandBy, over everything on the charger where the screen would have
    /// gone off; and the lock screen (a passcode set), over everything once
    /// it has. See faces.rs and lock.rs, and `Shell::key` for the order.
    faces: crate::faces::Faces,
    lock: crate::lock::Lock,
    /// What the lock screen shows besides the time, and when it was read.
    lock_info: Option<(crate::lock::Info, std::time::Instant)>,
    lock_scratch: Vec<u32>,
    /// What the island says once it is unlocked (the deep sleep's figures).
    after_unlock: Option<String>,
    /// The galleries' and tests' charger: plugged in or not, whatever the
    /// gauge says; and whether it was on the charger at the last tick.
    charger: Option<bool>,
    charger_was: bool,
    /// Live activities: music, timers, the queue, downloads… (liveactivity.rs).
    activities: crate::liveactivity::Activities,
    /// Notifications kept (notify.rs), and the robot's own as it tells them.
    pub notes: crate::notify::Store,
    robot_watch: crate::notify::RobotWatch,
    /// Tab held towards a lock: the cue round the island, 0 to 1, and
    /// whether the screen was lit at the last tick (a wake brings the lock
    /// screen's parts in again).
    hold_cue: Animated,
    lit_was: bool,
    /// The clipboard's history and quick capture, over whatever shows (shell/clip.rs).
    sheets: clip::Sheets,
}

/// How long the island holds a reminder coming due (a message holds 2.4 s).
const REMINDER_SECONDS: f32 = 6.0;

/// How long before the time to queue the island says it is coming.
const QUEUE_AHEAD: i64 = 300;

/// When `watch_queue` would next speak as things stand (from Nexus's queue
/// times): worked out ahead for deep standby, where the schedule is parked
/// and cannot change. The five-minute warning, then the time itself, for
/// whichever match is the next one by then.
fn queue_alert_at(view: &crate::schedule::View, now: i64, warned: &[(String, u8)]) -> Option<i64> {
    view.matches
        .iter()
        .filter(|m| m.score.is_none())
        .filter_map(|m| {
            let at = m.queue_at?;
            let said = |stage: u8| warned.iter().any(|(key, s)| *key == m.key && *s >= stage);
            let when = if !said(0) && now < at {
                (at - QUEUE_AHEAD).max(now)
            } else if !said(1) && now <= at + 900 {
                at.max(now)
            } else {
                return None;
            };
            (view.next(when).map(|n| n.key.as_str()) == Some(m.key.as_str())).then_some(when)
        })
        .min()
}

/// Music is playing (or about to): what it tells the live activities
/// (nowplaying), or what the player itself says (before Music sets
/// nowplaying, and while a stream is still being found).
fn music_playing() -> bool {
    crate::nowplaying::playing() || crate::music::playing()
}

/// A Clock timer is running: deep standby waits for it to be said.
#[cfg(not(test))]
fn timer_running() -> bool {
    crate::clock::any_running()
}

// In the tests a stand-in: Clock's timers are shared by every test at once.
#[cfg(test)]
thread_local! {
    static TIMER_RUNNING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn timer_running() -> bool {
    TIMER_RUNNING.with(std::cell::Cell::get)
}

#[derive(Clone, Copy)]
struct HelpHold {
    since: f32,
    last: f32,
    said: u32,
    opened: bool,
}

/// How long `?` is held to bring up the shortcut list, so a `?` typed in
/// passing never does.
const HOLD_FOR_SHORTCUTS: f32 = 5.0;
/// A held key repeats; a gap longer than this means it was let go. (The
/// first repeat comes 0.4 s after the press.)
const HELD_GAP: f32 = 0.5;

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
        Key::Delete => "del".into(),
        Key::Home => "home".into(),
        Key::End => "end".into(),
        Key::F(n) => format!("f{n}"),
        Key::BrightnessDown => "bright −".into(),
        Key::BrightnessUp => "bright +".into(),
        Key::Media(m) => crate::sound::key_name(m).into(),
        Key::Char(' ') => "space".into(),
        Key::Char(c) => c.to_string(),
        Key::Ctrl(' ') => "ctrl space".into(),
        Key::Ctrl(crate::platform::NOTIFICATIONS) => "ctrl bksp".into(),
        Key::Ctrl('\t') => "ctrl tab".into(),
        Key::Ctrl('\n') => "ctrl enter".into(),
        Key::Ctrl(c) if c.is_ascii_uppercase() => format!("ctrl shift {}", c.to_ascii_lowercase()),
        Key::Ctrl(c) => format!("ctrl {c}"),
        Key::Leave => "esc held".into(),
        Key::Lock => "tab held".into(),
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
    /// When the minute next turns: the clock is read again then, so it
    /// turns with Home's clock widget, not up to a few seconds after.
    turns: Option<std::time::Instant>,
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
            spotlight: crate::spotlight::Spotlight::new(),
            spotlight_under: None,
            spotlight_warmed: false,
            spent: Default::default(),
            tour: None,
            tour_after: None,
            places: crate::help::Places::default(),
            echo: Vec::new(),
            clock: 0.0,
            help_hold: None,
            recents: crate::recents::Recents::default(),
            switcher: None,
            switcher_under: None,
            saving: None,
            flash: None,
            voice: crate::voice::Talk::new(),
            island_rest: 60.0,
            // The island's words cross-fade on Bezel's effect spring; the title
            // giving way to it does too.
            title_shown: Animated::new(1.0, role::effect()).with_rest(0.004),
            status_shown: Animated::new(1.0, role::effect()).with_rest(0.004),
            status_left: W,
            standby: crate::standby::Standby::new(),
            care: crate::power::Care::new(),
            lights: crate::lights::Lights::new(),
            deep: crate::deep::Deep::new(),
            woke_by: None,
            queue_warned: Vec::new(),
            queue_age: 0.0,
            queue_due: None,
            reminders: crate::reminders::Watch::new(),
            woke_for_alert: false,
            faces: crate::faces::Faces::new(),
            lock: crate::lock::Lock::new(),
            lock_info: None,
            lock_scratch: Vec::new(),
            after_unlock: None,
            charger: None,
            charger_was: false,
            activities: crate::liveactivity::Activities::new(),
            notes: crate::notify::Store::kept(),
            robot_watch: crate::notify::RobotWatch::default(),
            hold_cue: Animated::new(0.0, role::smooth()).with_rest(0.004),
            lit_was: true,
            sheets: clip::Sheets::new(),
        }
    }

    /// `?` came, on a screen not taking text. Held for five seconds, it
    /// brings up the shortcut list; the island counts the last seconds
    /// down. A `?` pressed and let go does nothing.
    fn question_mark(&mut self) {
        let now = self.clock;
        let mut hold = match self.help_hold {
            Some(h) if now - h.last <= HELD_GAP => HelpHold { last: now, ..h },
            _ => HelpHold { since: now, last: now, said: 0, opened: false },
        };
        // Over the list itself, holding ? does nothing more.
        if self.top_title() == "Shortcuts" {
            hold.opened = true;
        }
        let held = now - hold.since;
        if !hold.opened && held >= HOLD_FOR_SHORTCUTS {
            hold.opened = true;
            self.help_hold = Some(hold);
            self.message = None;
            return self.open(Box::new(crate::help::Shortcuts::new()), Path::Slide);
        }
        // Held a second or more: say what holding on will do.
        let left = (HOLD_FOR_SHORTCUTS - held).ceil() as u32;
        if !hold.opened && held >= 1.0 && left != hold.said {
            hold.said = left;
            self.say(format!("shortcuts in {left} s"));
        }
        self.help_hold = Some(hold);
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
            // Over a screen drawn live: recent apps on their way out go at once.
            if self.switcher.as_ref().is_some_and(|s| s.leaving()) {
                (self.switcher, self.switcher_under) = (None, None);
            }
        }
        self.control.set_open(open, &self.settings, &self.notes);
        self.dirty = true;
    }

    #[cfg(test)]
    pub fn control_open(&self) -> bool {
        self.control.is_open()
    }

    /// Something said in the island for a moment (a volume, a screenshot
    /// saved, an app's word): gone once said. What is worth keeping is a
    /// notification (`notify`).
    pub fn say(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), MESSAGE_SECONDS));
        self.dirty = true;
    }

    /// A notification: kept (the notification centre, the lock screen, the
    /// island's count), and said in the island, with the Fn light blinking
    /// until the screen wakes if it is off. Unsaid with Quiet on (what
    /// breaks through aside), or when its app is in front (it shows it).
    pub fn notify(&mut self, n: crate::notify::Notification) {
        use crate::standby::State;
        let awake = self.standby.state() == State::Awake && !self.lock.covering() && !self.faces.showing();
        let front = awake && !n.urgent && self.stack.last().is_some_and(|(s, _)| s.app() == n.app);
        let (said, urgent) = (n.said(), n.urgent);
        self.notes.add(n, front);
        self.dirty = true;
        if front || (self.notes.quiet() && !urgent) {
            return;
        }
        self.message = Some((said, MESSAGE_SECONDS));
        if self.standby.state() == State::Off {
            self.lights.notice();
        }
    }

    /// The notification centre: the control centre's sheet, down and turned
    /// to the notifications (or put away, if it shows them already).
    fn show_notifications(&mut self) {
        if self.control.is_open() && self.control.page() == crate::control::Page::Notices {
            return self.set_control(false);
        }
        if !self.control.visible() {
            self.control_under = None;
            if self.switcher.as_ref().is_some_and(|s| s.leaving()) {
                (self.switcher, self.switcher_under) = (None, None);
            }
        }
        self.control.turn_to(crate::control::Page::Notices, &self.settings, &mut self.notes);
        self.moving = true;
        self.dirty = true;
    }

    /// Where a notification leads, opened over the screen in front; it is
    /// put away as it opens.
    fn open_notification(&mut self, id: u64) {
        let Some(n) = self.notes.dismiss(id) else { return };
        let Some(action) = n.leads_to() else { return };
        let mut say = None;
        let opened = crate::notify::open(&action, &mut Cx { robot: &mut self.robot, settings: &mut self.settings, say: &mut say, at_rest: true });
        match opened {
            Ok(screen) => self.open(screen, Path::Slide),
            Err(why) => self.say(why),
        }
        self.moving = true;
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

    /// The screen in front is at work (a question being answered): the
    /// Assistant's gallery waits for it to finish.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn top_busy(&self) -> bool {
        self.stack.last().is_some_and(|(screen, _)| screen.busy())
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
        // Locked at start, Home is not what appears: no greeting, and none
        // kept as a notice either.
        if self.settings.greeting && !self.lock.engaged() {
            use chrono::Timelike;
            let team = crate::schedule::team_of(&self.robot.address);
            self.say(greeting(chrono::Local::now().hour(), team));
        }
    }

    pub fn key(&mut self, key: Key) {
        self.dirty = true;
        self.last_key = std::time::Instant::now();
        // The sound and music keys work with the screen off (music in a
        // pocket) without waking it, and without a word for nobody to read:
        // in music standby too. In deep standby nothing plays and the
        // radios are off: there only the volume keys stay asleep, and play
        // wakes it like any key.
        if let Key::Media(media) = key {
            let volume = matches!(media, crate::platform::Media::Mute | crate::platform::Media::VolumeDown | crate::platform::Media::VolumeUp);
            if (self.deep.on() && (volume || self.deep.music())) || (!self.deep.on() && self.standby.state() == crate::standby::State::Off) {
                crate::sound::media_key(media);
                return;
            }
        }
        // Tab held: the lock screen, from anywhere (the screen woken, StandBy
        // or deep standby left for it). Locked already, it does nothing.
        if key == Key::Lock {
            return self.lock_by_hand();
        }
        // The key that woke it from deep standby, still held: its repeats
        // are spent too, so none reaches the app as it wakes.
        if let Some((woke, last)) = self.woke_by.take() {
            if key == woke && self.clock - last <= HELD_GAP {
                self.woke_by = Some((key, self.clock));
                return;
            }
        }
        // Any key wakes it from deep standby, and is spent doing so.
        if self.deep.on() {
            self.standby.key();
            self.leave_deep();
            self.woke_by = Some((key, self.clock));
            return;
        }
        // StandBy: the arrows turn its faces; any other key leaves it below,
        // as a key wakes the screen from off.
        if self.standby_state() == crate::standby::State::Faces {
            // Music's keys and the volume work on StandBy without leaving it.
            if let Key::Media(media) = key {
                self.media_on_lock(media);
                self.moving = true;
                return;
            }
            let before = self.settings.lock;
            if self.faces.key(key, &mut self.settings.lock, self.settings.calm) {
                if self.settings.lock != before {
                    crate::save_settings(&self.settings);
                    self.gfx.lock = self.settings.lock;
                }
                self.moving = true;
                return;
            }
        }
        // A key while the screen is dimmed or off wakes it, and is spent doing so.
        if self.standby.key() {
            self.leave_faces();
            self.moving = true;
            return;
        }
        // Voice: any key stops an answer being said (Esc does nothing else),
        // and Esc lets go of what is being listened to or written down.
        if self.voice.key(key) {
            self.moving = true;
            return;
        }
        // Developer options › Show key presses: every key, as it lands (but
        // never a passcode's, on the lock screen).
        let secret = self.lock.engaged() || self.stack.last().is_some_and(|(s, _)| s.secret());
        if self.settings.developer && self.settings.show_keys && !secret {
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
        // Locked: the lock screen takes every key. Nothing below this line
        // (the tour, the recent apps, the control centre, Spotlight, `?`, any
        // shortcut, the screen in front) is reached until the passcode is.
        // Shown by hand with no passcode, any key lifts it, and is spent
        // doing so. Music's keys (Fn + Q, W, E) and the volume work on it.
        if self.lock.covering() {
            if let Key::Media(media) = key {
                self.media_on_lock(media);
            } else {
                self.lock.key(key);
            }
            self.moving = true;
            return;
        }
        // The tour takes every key while it shows.
        if let Some(tour) = &mut self.tour {
            tour.key(key);
            self.moving = true;
            return;
        }
        // The clipboard's history (Ctrl-Shift-H) and quick capture
        // (Ctrl-Shift-N) come up from anywhere past the lock, a terminal
        // included, and take the keys while they show (shell/clip.rs).
        if self.sheets_key(key) {
            self.moving = true;
            return;
        }
        // The recent apps take the keys while they show. On their way out
        // they go on going, over the screen drawn live, and the key lands
        // on that screen.
        if self.switcher.as_ref().is_some_and(|s| !s.leaving()) {
            return self.switcher_key(key);
        }
        // Spotlight on its way back into the island: Ctrl-Enter turns it
        // round; any other key makes it go at once, and lands on the screen
        // beneath, drawn live.
        if !self.spotlight.is_open() && self.spotlight.visible() {
            if key == Key::Ctrl('\n') {
                return self.summon_spotlight();
            }
            self.spotlight.gone_now();
            self.spotlight_under = None;
        }
        // Spotlight takes the keys while it is up (the backlight's aside).
        if self.spotlight.is_open() && !matches!(key, Key::BrightnessDown | Key::BrightnessUp) {
            return self.spotlight_key(key);
        }
        // Ctrl-Tab brings them up over any screen, a terminal included: no
        // program in a terminal can be sent Ctrl-Tab, so it takes nothing.
        if key == Key::Ctrl('\t') {
            return self.open_recents(None);
        }
        // Fn + U and Fn + I: the backlight, over any screen.
        if matches!(key, Key::BrightnessDown | Key::BrightnessUp) {
            let step: i32 = if key == Key::BrightnessUp { 10 } else { -10 };
            let said = match crate::sys::brightness() {
                Some(now) if crate::sys::set_brightness((now as i32 + step).clamp(10, 100) as u32) => {
                    // Kept, for the next start.
                    self.settings.brightness = crate::sys::brightness().unwrap_or(now);
                    crate::save_settings(&self.settings);
                    format!("brightness {}%", self.settings.brightness)
                }
                _ => "the backlight is on the handheld".to_owned(),
            };
            return self.say(said);
        }
        // Fn + A, S, D (mute, quieter, louder) and Fn + Q, W, E (play or
        // pause, previous, next), over any screen (sound.rs, music.rs).
        if let Key::Media(media) = key {
            if let Some(said) = crate::sound::media_key(media) {
                self.say(said);
            }
            // Music's own keys open the island on what is playing.
            if matches!(media, crate::platform::Media::PlayPause | crate::platform::Media::Next | crate::platform::Media::Previous) {
                self.activities.refresh();
                self.activities.show(crate::liveactivity::Kind::Music, self.settings.calm);
            }
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
        // Ctrl-Enter brings Spotlight up over any screen, a terminal included
        // (no program can tell it from Enter), and over the control centre,
        // which gives way to it.
        if key == Key::Ctrl('\n') {
            return self.summon_spotlight();
        }
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
            match self.control.key(plain(key), &mut self.settings, &mut self.notes) {
                None => self.set_control(false),
                Some(Outcome::Settings) => {
                    self.apply_settings();
                    crate::save_settings(&self.settings);
                    // The screen behind changed its look: take it again.
                    self.control_under = None;
                }
                // The recent apps, over the screen the sheet came down on.
                Some(Outcome::Recents) => {
                    let under = self.control_under.take();
                    self.control.set_open(false, &self.settings, &self.notes);
                    self.control.shown.jump(0.0);
                    self.control_glass = None;
                    self.open_recents(under);
                }
                // Where a notification leads: the sheet goes as it opens.
                Some(Outcome::Open(id)) => {
                    self.set_control(false);
                    self.open_notification(id);
                }
                // A picture of the screen beneath the sheet, as the sheet goes.
                Some(Outcome::Screenshot) => {
                    let picture = self.control_under.clone();
                    self.set_control(false);
                    // Never of the vault open: the picture would keep on the
                    // card, in plain sight, what the vault seals.
                    if crate::vault::is_open() {
                        self.say("no screenshots of the vault");
                    } else if let Some(picture) = picture {
                        self.screenshot(picture);
                    }
                }
                Some(Outcome::Nothing) => {}
            }
            return;
        }
        // Ctrl-Space brings it down over any screen, a terminal included;
        // Ctrl-Backspace brings it down turned to the notifications.
        if key == Key::Ctrl(' ') {
            return self.set_control(true);
        }
        if key == Key::Ctrl(crate::platform::NOTIFICATIONS) {
            return self.show_notifications();
        }
        // ? held for five seconds brings the shortcut list up over any
        // screen that is not taking text (in a terminal or a text field, it
        // types as ever).
        if key == Key::Char('?') {
            let (top, _) = self.stack.last().expect("the home screen is always there");
            if !top.wants_text() && !top.full_screen() {
                return self.question_mark();
            }
        }
        // The living wallpaper: a move sets its light drifting the same way,
        // and it settles back still. (Held still by the battery saver: each
        // drift is a run of frames.)
        if !self.settings.calm && !self.settings.battery.saver {
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
        // Copy and paste, into and out of the screen in front (shell/clip.rs).
        let Some(key) = self.clipboard_key(key) else { return };
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
            Nav::Spotlight => self.summon_spotlight(),
            Nav::Done => self.finish_top(),
            Nav::Tour => self.tour_from_home(),
            Nav::Home => self.back_home(),
        }
    }

    /// Back to Home from however deep: the screens above it fade away over
    /// it, as the tour's start does.
    fn back_home(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
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
        self.moving = true;
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
        if self.gfx.widgets != self.settings.widgets {
            self.gfx.widgets.clone_from(&self.settings.widgets);
        }
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
        (self.gfx.screen_off, self.gfx.power) = (self.settings.screen_off, self.settings.power);
        self.gfx.battery = self.settings.battery;
        // The battery saver has the processor climb slowly, whatever is chosen.
        self.standby.set_power(if self.settings.battery.saver { crate::standby::Power::Saving } else { self.settings.power });
        (self.gfx.lift_wake, self.gfx.face_down) = (self.settings.lift_wake, self.settings.face_down);
        self.gfx.lock = self.settings.lock;
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
                t.over = self.stack.len();
                self.stack.push((returning, path));
                t.progress.set(1.0);
                return;
            }
        }
        // A page pushed before the screen arriving was ever drawn (the keys
        // came before a frame): it comes in with that screen, in its window,
        // rather than sliding over a screen that never showed.
        if path == Path::Slide && self.transition.as_ref().is_some_and(Transition::unseen) && !self.settings.calm {
            self.push(screen, path);
            self.gfx.reveal_start();
            return;
        }
        // Whatever was still on its way in or out goes on beneath the screen
        // arriving, from where it was and at its speed: a page pushed as the
        // last slides in, an app opened as another goes back into its tile.
        let beneath = self.interrupt();
        // An app opened from Home that was left a moment ago comes back as it
        // was (its pages, its place): the new copy is not needed. It closes
        // into the tile it was opened from this time.
        if let (Path::Grow(from), 1) = (path, self.stack.len()) {
            if let Some(mut recent) = self.recents.find(screen.title()).and_then(|at| self.recents.take(at)) {
                drop(screen);
                recent.stack[0].1 = path;
                // Pages deep, it grows over Home rather than over its own first page.
                let home = (recent.stack.len() > 1 && self.last_depth == 1).then(|| self.last_screen.0.clone());
                self.bring_back(recent, from, home);
                if let Some(t) = &mut self.transition {
                    t.beneath = beneath;
                }
                return;
            }
        }
        let over = self.stack.len();
        self.push(screen, path);
        self.gfx.reveal_start();
        if self.settings.calm {
            self.transition = None;
            return;
        }
        let mut transition = Transition::new(path, 0.0, 1.0, None, None);
        (transition.beneath, transition.over) = (beneath, over);
        self.transition = Some(transition);
    }

    /// A screen onto the stack. A picture kept of the screen last drawn at
    /// its depth (or deeper) is of another screen now, and is not used again.
    fn push(&mut self, screen: Box<dyn Screen>, path: Path) {
        if self.last_depth > self.stack.len() {
            self.last_depth = usize::MAX;
        }
        self.stack.push((screen, path));
    }

    /// The transition under way gives way to a new one. What it was taking
    /// away is let go now (an app goes among the recent apps); its motion,
    /// if it has been seen and still has somewhere to go, is handed back as
    /// a ghost to play on beneath or within the new one. Called before the
    /// stack changes for the new one.
    fn interrupt(&mut self) -> Option<Box<Transition>> {
        let mut t = self.transition.take()?;
        if let Some(mut screen) = t.leaving.take() {
            // Turned back as it rose: what is left of its rise goes with it.
            self.gfx.reveal_stop();
            // Not drawn leaving yet (the keys came before a frame): it goes
            // from the picture it last showed, over the screen it leaves for.
            if t.pictures.is_none() {
                let over = self.leaving_picture(screen.as_mut());
                t.pictures = Some([self.top_picture(), over]);
                t.made = PICTURES_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            }
            let picture = t.pictures.as_ref().map(|[_, (over, _)]| over.clone());
            self.left(screen, t.path, picture);
        } else if t.pictures.is_none() && !t.unseen() && !t.ghost {
            // Arriving, undrawn, but showing all the same (asked back before
            // a frame, from where its last motion had it): its pictures now.
            let base = if (1..self.stack.len()).contains(&t.over) { t.over } else { self.stack.len() - 1 };
            let under = match self.last_depth == base {
                true => self.last_screen.clone(),
                false => Self::picture_of(&mut self.gfx, &self.background, &self.robot, self.stack[base - 1].0.as_mut(), base == 1),
            };
            t.pictures = Some([under, self.top_picture()]);
        }
        // Over a picture of what showed rather than the screen beneath (an
        // app from the recent apps): no other motion can go on over that,
        // so the panel as it showed fades into what comes next instead.
        if t.over_backdrop() {
            self.fade_out_shown();
            return None;
        }
        let still = t.progress.is_settled() && t.beneath.is_none() && t.within.is_none();
        if t.pictures.is_none() || still {
            return None;
        }
        t.ghost = true;
        t.prune(2, &self.background, self.gfx.tile_radius(), self.gfx.tokens.ground);
        // Its pictures may have changed: the GPU takes them up afresh.
        t.made = PICTURES_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        Some(Box::new(t))
    }

    fn close(&mut self) {
        let depth = self.stack.len();
        if depth <= 1 {
            return;
        }
        self.gfx.focus_reset();
        if let Some(t) = self.transition.as_mut().filter(|t| t.arriving() && !self.settings.calm) {
            let (screen, path) = self.stack.pop().expect("more than one screen");
            if let Some((below, _)) = self.stack.last_mut() {
                below.resume();
            }
            // Still arriving: turn it round where it is, keeping its speed.
            // Its rows go on rising as it goes, so asked back it still has them.
            if self.stack.len() == t.over {
                t.leaving = Some(screen);
                t.progress.set(0.0);
                return;
            }
            self.gfx.reveal_stop();
            // Not the screen it brings in but a page over it (an app brought
            // back pages deep): the app goes on arriving, and the page slides
            // away inside its window. Not yet seen, the page simply goes.
            if let Some([_, (over, _)]) = &mut t.pictures {
                let depth = self.stack.len();
                let (under, dots) = Self::picture_of(&mut self.gfx, &self.background, &self.robot, self.stack[depth - 1].0.as_mut(), depth == 1);
                let mut page = Transition::new(path, 1.0, 0.0, None, None);
                page.ghost = true;
                page.pictures = Some([(under.clone(), dots), (std::mem::replace(over, under), None)]);
                // A page above it still going (Esc again): on, over this one.
                page.within = t.within.take();
                t.within = Some(Box::new(page));
                t.made = PICTURES_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            }
            return self.left(screen, path, None);
        }
        // Anything else under way (a page still on its way out as Esc comes
        // again) goes on within the picture of the screen now leaving.
        self.gfx.reveal_stop();
        let mut within = self.interrupt();
        let (screen, path) = self.stack.pop().expect("more than one screen");
        if let Some((below, _)) = self.stack.last_mut() {
            below.resume();
        }
        if self.settings.calm {
            let picture = (self.last_depth == self.stack.len() + 1).then(|| self.last_screen.0.clone());
            return self.left(screen, path, picture);
        }
        let mut transition = Transition::new(path, 1.0, 0.0, Some(screen), None);
        // Its own opening, still playing beneath the page it was leaving
        // for: that is turned round from where it is, with its speed, over
        // whatever went on beneath it.
        if let Some(opening) = within.as_mut().and_then(|w| w.take_opening(path)) {
            transition.progress = opening.progress;
            transition.progress.set(0.0);
            transition.beneath = opening.beneath;
        }
        transition.within = within;
        self.transition = Some(transition);
    }

    /// An external app to hand the panel to: the top screen asks for one.
    /// Only once its page has landed and shows alone (nothing on its way in
    /// or out, no recent apps, control centre, tour or fade over it, the
    /// screen awake), so the app starts on a settled picture and no key
    /// meant for something over it reaches the app.
    pub fn take_launch(&mut self) -> Option<crate::apps::App> {
        let alone = self.transition.is_none()
            && self.switcher.is_none()
            && !self.control.visible()
            && self.tour.is_none()
            && self.intro.is_none()
            && self.curtain.is_none()
            && self.standby.state() == crate::standby::State::Awake
            // Never while locked (or StandBy shows): an app handed the panel
            // would take the keys from under the lock. `locked` also counts
            // a lock Spotlight holds and a screen that locks.
            && !self.locked()
            && !self.faces.visible();
        if !alone {
            return None;
        }
        self.stack.last_mut().and_then(|(screen, _)| screen.launch())
    }

    /// The app that had the panel is done: say so if it failed, and close
    /// the page it was opened from. `last` is what the app left on the
    /// panel, where it can be read back: the window closes from that
    /// picture into its tile, rather than from the page it was opened over.
    pub fn launch_ended(&mut self, result: Result<(), String>, last: Option<Vec<u32>>) {
        if let Err(why) = result {
            self.say(why);
        }
        if let Some(picture) = last.filter(|px| px.len() == self.gfx.canvas.px.len()) {
            self.gfx.canvas.px.copy_from_slice(&picture);
            self.last_screen = (picture, None);
            self.last_depth = self.stack.len();
        }
        self.close();
        // Drawn at once: the app's last picture does not stay on the panel
        // while the shell waits out an idle spell.
        self.moving = true;
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
                // A robot plugged in wakes the screen to show it.
                self.standby.wake_now();
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
        let on_home = self.stack.len() == 1 && self.transition.is_none() && !self.control.visible() && !self.spotlight.visible();
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
        let name = self.robot.string("Robot/Identity/Name").unwrap_or_else(|| "robot".into()).to_lowercase();
        let linked = crate::notify::Notification::new("Robot", "smart_toy", format!("{name} linked")).tag("robot:link").action("builtin:Robot");
        if self.message.is_none() {
            self.notify(linked);
        } else {
            self.notes.add(linked, false);
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
                self.notify(crate::notify::Notification::said_as("System update", "download", &words).tag("update").action("setting:System update"));
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
        // The first start after an update, once it is kept: what changed,
        // once, sliding in over Home when Home is at rest and in sight.
        if let Some(version) = crate::changelog::due_after_update(&self.update) {
            let at_rest = self.stack.len() == 1 && self.transition.is_none() && self.curtain.is_none() && self.intro.is_none();
            let clear = self.tour.is_none() && self.tour_after.is_none() && self.switcher.is_none() && !self.control.is_open() && !self.spotlight.visible();
            if at_rest && clear && !self.standby.asleep() && crate::setup_done() {
                if let Some(page) = crate::changelog::WhatsNew::after_update(&version) {
                    self.open(Box::new(page), Path::Slide);
                }
                crate::changelog::mark_shown(&version);
            }
        }
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
        let busy = watch.touched || self.control.visible() || self.spotlight.visible() || self.transition.is_some();
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
            self.notify(crate::notify::Notification::new("Logs", "insights", format!("how {short} went")).body("its summary is in Logs").action(format!("log:{}", row.name)));
            return;
        }
        self.say(format!("how {short} went"));
        self.open(Box::new(detail), Path::Slide);
    }

    /// Advance time. Returns whether a frame should be drawn.
    pub fn tick(&mut self, dt: f32) -> bool {
        // Deep standby: the shell looks in once a minute (`dt` is the time
        // that passed), for the battery and the red light's reminder.
        let mut dt = dt;
        if self.deep.on() {
            self.clock += dt;
            // The talk key (Alt) wakes it as any key does, and is spent doing so.
            if crate::voice::talk_key_down() {
                self.standby.key();
                self.voice.spend();
            }
            self.tick_deep(dt);
            if self.deep.on() && self.standby.asleep() {
                return false;
            }
            // Woken by something other than a key (the battery running
            // out); or music standby gave way to plain standby, the screen
            // still dark, and the shell ticks as it does there.
            if self.deep.on() {
                self.leave_deep();
            }
            dt = dt.min(0.25);
        }
        if self.deep.waking() {
            self.finish_waking();
        }
        self.clock += dt;
        // `?` let go: its hold is over, and a countdown it left in the island goes.
        if let Some(hold) = self.help_hold.filter(|h| self.clock - h.last > HELD_GAP) {
            self.help_hold = None;
            if !hold.opened && self.message.as_ref().is_some_and(|(text, _)| text.starts_with("shortcuts in")) {
                self.message = None;
                self.dirty = true;
            }
        }
        self.gfx.tick_gpu();
        self.watch_link(dt);
        // The look and the team Catalyst Companion sent, put in place.
        if crate::companion::apply_pending(&mut self.robot, &mut self.settings) {
            self.apply_settings();
        }
        // Motion only moves forward in time if it was already moving; slower
        // under developer options' animation scale.
        let motion_dt = if self.moving { dt.min(0.05) / self.motion_scale() } else { 0.0 };
        let mut moving = false;
        // The tour: offered a moment after setup, once Home has arrived.
        if let Some(left) = &mut self.tour_after {
            *left -= dt;
            if *left <= 0.0 && self.transition.is_none() && self.curtain.is_none() && self.stack.len() == 1 && self.intro.is_none() && !self.lock.visible() && !self.faces.visible() {
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
            // Both must be advanced, so no short-circuit.
            if t.progress.tick(motion_dt) | t.tick_nested(motion_dt) || motion_dt == 0.0 {
                moving = true;
            } else {
                let ended = self.transition.take().expect("in flight");
                // Turned back and gone: whatever was left of its rise goes
                // with it (the screen it lands on is not rising).
                if ended.closing() {
                    self.gfx.reveal_stop();
                }
                // Gone back into its card (or back before it ever showed
                // growing out of it): the picture of the cards fades into
                // the screen that is really beneath.
                if ended.closing() && ended.over_backdrop() {
                    moving |= self.fade_out_shown();
                }
                // The living wallpaper: once a screen has come or gone, its
                // light drifts the way it went, and settles still.
                if !self.settings.calm && !self.settings.battery.saver {
                    let closing = ended.closing();
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
                // An app gone back to Home is kept among the recent apps,
                // with its picture (one never drawn leaving: as it showed).
                if let Some(mut screen) = ended.leaving {
                    let picture = match ended.pictures {
                        Some([_, (over, _)]) => over,
                        None => self.leaving_picture(screen.as_mut()).0,
                    };
                    self.left(screen, ended.path, Some(picture));
                }
            }
        }
        moving |= self.tick_recents_and_standby(dt, motion_dt);
        moving |= self.tick_voice(dt);
        moving |= self.title_shown.tick(motion_dt) | self.status_shown.tick(motion_dt);
        self.tick_power(dt);
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
        moving |= self.tick_spotlight(motion_dt);
        moving |= self.tick_sheets(dt, motion_dt);
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
            } else if SETTINGS_CHANGED.swap(false, std::sync::atomic::Ordering::Relaxed) {
                // A screen changed the settings with no key pressed (the
                // assistant, when a tool comes back): drawn as they are now.
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
        // Enter held keeps sending a remote's button; anywhere else (the
        // control centre or the recent apps over it included) it counts once.
        let enter_repeats = self.transition.is_none()
            && !self.control.is_open()
            && !self.spotlight.is_open()
            && !self.sheets.open()
            && self.switcher.is_none()
            && self.tour.is_none()
            && self.stack.last().is_some_and(|(s, _)| s.repeats_enter());
        crate::platform::ENTER_REPEATS.store(enter_repeats, std::sync::atomic::Ordering::Relaxed);
        if let Some((_, left)) = &mut self.message {
            *left -= dt;
            if *left <= 0.0 {
                self.message = None;
                self.dirty = true;
            }
        }
        // The clock and the battery, a few times a minute (every second
        // while the clock shows seconds), and as the minute turns.
        self.status.age += dt;
        let turned = self.status.turns.is_some_and(|at| std::time::Instant::now() >= at);
        if turned || self.status.age > if self.settings.seconds { 1.0 } else { 5.0 } {
            self.status.age = 0.0;
            self.gfx.look_for_gpu();
            let now = chrono::Local::now();
            let left = 60.0 - chrono::Timelike::second(&now) as f64 - chrono::Timelike::nanosecond(&now).min(999_999_999) as f64 / 1e9;
            self.status.turns = Some(std::time::Instant::now() + std::time::Duration::from_secs_f64(left.max(0.0) + 0.01));
            let clock = self.settings.clock(now);
            let battery = self.care.gauge_percent().or_else(crate::sys::battery_percent);
            let (wired, wireless) = (crate::sys::link_up("eth0"), crate::sys::link_up("wlan0"));
            if (clock.as_str(), battery, wired, wireless) != (self.status.clock.as_str(), self.status.battery, self.status.wired, self.status.wireless) {
                self.status = Status { clock, battery, wired, wireless, age: 0.0, turns: self.status.turns };
                self.dirty = true;
            }
        }
        self.watch_update(dt);
        let changed = self.robot.refresh(dt);
        // The robot's own notifications, looked for only as its values change.
        if changed {
            if let Some(n) = self.robot_watch.look(&self.robot) {
                self.notify(n);
            }
        }
        // Music standby, half a minute into the screen being dark with
        // music playing; deep standby, once the screen has been off long
        // enough with no key. Neither if something would be cut off.
        if self.deep.music_due(self.standby.asleep(), music_playing(), dt) {
            if self.music_blocker().is_none() {
                self.enter_music_standby();
            }
        } else if self.standby.asleep() && self.deep.due(self.standby.idle(), self.settings.battery.deep_after, dt) && self.deep_blocker().is_none() {
            self.enter_deep();
        }
        // The page's own ambient change (the field's robot, a replay
        // playing), in real time: drawn, but not counted as motion.
        let ambient = self.transition.is_none() && !self.standby.asleep() && self.stack.last_mut().is_some_and(|(s, _)| s.ambient_tick(dt, &self.robot));
        // With the screen off and dark, nothing is drawn (or sent to the panel).
        let draw = (moving || changed || ambient || self.dirty) && !self.standby.asleep();
        self.dirty = false;
        self.moving = moving;
        draw
    }

    /// Something drawn a few times a second that is not motion (Music's
    /// bars on the island, the lock screen's robot turning on its stand,
    /// the field's robot): how often the main loop looks in for it.
    pub fn ambient(&self) -> Option<std::time::Duration> {
        if self.standby.asleep() {
            return None;
        }
        // My own tuning: 15 a second, lively enough for bars that follow
        // the music, and about a quarter of the frames (and the panel's
        // time, as only the island's rows change) of keeping to the beat.
        let bars = self.activities.bars_live.then_some(std::time::Duration::from_millis(66));
        let robot = (self.lock.drifting() && !self.faces.visible()).then_some(crate::depth::DRIFT_EVERY);
        let page = if self.transition.is_none() { self.stack.last().and_then(|(s, _)| s.ambient()) } else { None };
        bars.into_iter().chain(robot).chain(page).min()
    }

    /// Whether something on screen is in motion.
    pub fn moving(&self) -> bool {
        self.moving
    }

    /// Whether to keep to the frame beat: something is moving, or the
    /// screen in front wants looking at often.
    pub fn animating(&self) -> bool {
        // With the screen off and dark, nothing keeps the frame beat: the
        // shell looks in a few times a second, and the processor rests.
        !self.standby.asleep() && (self.moving || self.stack.last().is_some_and(|(s, _)| s.busy()))
    }

    /// How far the transition in flight has come, if there is one.
    #[cfg(test)]
    fn progress(&self) -> Option<f32> {
        self.transition.as_ref().map(|t| t.progress.value())
    }

    pub fn draw(&mut self) {
        // Music's bars run only while the island shows them: it says so as it draws.
        self.activities.bars_seen = false;
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
        self.draw_hold_cue();
        self.draw_flash_and_standby();
    }

    /// Tab held towards a lock: a ring of the signal colour drawn round the
    /// island from its top, clockwise, filling as Tab is held; full, the
    /// lock screen comes down. Let go early, it draws back.
    fn draw_hold_cue(&mut self) {
        let p = self.hold_cue.value().clamp(0.0, 1.0);
        if p <= 0.004 {
            return;
        }
        let island = self.places.island.filter(|_| !self.stack.last().is_some_and(|(s, _)| s.full_screen())).unwrap_or_else(|| {
            let h = self.gfx.island_h();
            Rect::new((W - 64.0) * 0.5, 4.0, 64.0, h)
        });
        let r = island.inset(-2.5);
        let path = capsule_path(r, p);
        let g = &mut self.gfx;
        let color = g.tokens.signal;
        // A soft glow under the line, then the line; both fading in over the
        // first part so the start is never a hard dot.
        let fade = (p / 0.12).min(1.0);
        g.canvas.polyline(&path, 4.0, color, 0.18 * fade);
        g.canvas.polyline(&path, 1.8, color, 0.95 * fade);
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
        // StandBy or the lock screen over the whole panel: nothing beneath
        // them is drawn.
        let covered = self.faces.covers() || (self.lock.covers() && !self.faces.visible());
        if !covered {
            self.draw_shell();
        }
        self.draw_lock_and_faces();
    }

    /// The lock screen and StandBy, over whatever is beneath them.
    fn draw_lock_and_faces(&mut self) {
        if self.lock.visible() && !self.faces.covers() {
            let info = self.lock_info();
            let Shell { gfx, lock, settings, lock_scratch, .. } = self;
            lock.draw(gfx, &settings.lock, &info, lock_scratch);
            // The field widening to say something.
            if self.gfx.take_motion_started() {
                self.moving = true;
            }
        }
        if self.faces.visible() {
            let reading = self.shown_reading();
            let feed = crate::faces::Feed { reading: reading.as_ref(), h24: self.settings.clock24, notices: self.notes.unread(), generation: self.robot.schedule.generation(), activities: self.activities.list(), opened: self.activities.opened(), open: self.activities.open.value(), level: self.activities.level.value() };
            let Shell { gfx, faces, robot, settings, .. } = self;
            faces.draw(gfx, robot, &feed, &settings.lock);
            // The robot clock's robot taking up a new pose.
            if self.gfx.take_motion_started() {
                self.moving = true;
            }
        }
    }

    fn draw_shell(&mut self) {
        // The recent apps, over the wallpaper alone (no screen's words show
        // through between the cards): the screen in front fades into it as
        // they come, and back as they go. Both are held still while nothing
        // beneath moves; drawn live while a screen is still on its way in or
        // out behind the cards, and as the cards go (a key may already have
        // reached the screen), so nothing beneath freezes or jumps.
        if let Some(leaving) = self.switcher.as_ref().map(|s| s.leaving()) {
            let live = self.transition.is_some() || leaving;
            if self.switcher_under.is_none() || live {
                self.draw_frame();
                let (wall, frame) = (&self.background, &self.gfx.canvas.px);
                match &mut self.switcher_under {
                    Some((held_wall, front)) if held_wall.len() == wall.len() && front.len() == frame.len() => {
                        held_wall.copy_from_slice(wall);
                        front.copy_from_slice(frame);
                    }
                    held => *held = Some((wall.clone(), frame.clone())),
                }
            }
            if let (Some(switcher), Some((wall, front))) = (&mut self.switcher, &self.switcher_under) {
                switcher.draw(&mut self.gfx, wall, front);
            }
            return;
        }
        let over = self.control.visible();
        // The clipboard's history or quick capture, as Spotlight below (shell/clip.rs).
        if !over && self.draw_sheets() {
            return;
        }
        // Spotlight is up: the screen behind is held as it was (drawn live
        // only while a screen is still on its way), and the bar grows over it.
        if self.spotlight.visible() && !over {
            if self.transition.is_some() {
                self.spotlight_under = None;
            }
            match &self.spotlight_under {
                Some((under, pager)) => {
                    self.gfx.canvas.px.copy_from_slice(under);
                    self.gfx.pager = *pager;
                }
                None => {
                    // Drawn without the island's glass: the bar is drawn with it.
                    self.draw_frame();
                    if self.transition.is_none() {
                        self.spotlight_under = Some((self.gfx.canvas.px.clone(), self.gfx.pager));
                    }
                }
            }
            return self.draw_spotlight();
        }
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
        let Shell { gfx: g, control, control_glass, notes, .. } = self;
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
        control.draw(g, notes);
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
                // Motion played beneath or within it was drawn over the old
                // wallpaper, and cannot be drawn again: it lands.
                (transition.beneath, transition.within) = (None, None);
            }
            if self.transition.as_ref().is_some_and(|t| t.ghost) {
                self.transition = None;
            }
        }
        g.canvas.px.copy_from_slice(&self.background);
        g.pager = None;
        g.pager_at = None;
        // A title runs up to the island as it rests. An island grown wider
        // than that (a message, push-to-talk) passes over the title, which
        // fades out under its glass and back, rather than being cut shorter.
        g.title_room = (W - self.island_rest.max(g.island_h())) * 0.5 - PAD - 8.0;
        g.title_alpha = self.title_shown.value().clamp(0.0, 1.0);
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
                    g.pager_at = None;
                    (g.canvas.px.clone(), g.pager.take().map(|(pages, current, _)| (pages, current)))
                };
                // The screen that was showing was drawn last frame. Keeping that
                // picture means a transition's first frame draws one screen,
                // not two, so it starts without a stall.
                let kept = |depth_then: usize| (self.last_depth == depth_then).then(|| self.last_screen.clone());
                transition.backdropped |= transition.backdrop.is_some();
                transition.pictures = Some(match &mut transition.leaving {
                    Some(leaving) => {
                        let over = kept(depth + 1).unwrap_or_else(|| picture(g, leaving.as_mut(), false, false));
                        // Turned back before it was drawn growing out of its
                        // card: it goes back over the cards as they showed.
                        let under = match transition.backdrop.take() {
                            Some(backdrop) => (backdrop, None),
                            None => picture(g, self.stack[depth - 1].0.as_mut(), depth == 1, false),
                        };
                        [under, over]
                    }
                    None => {
                        // Over the screen it arrived over, which is not the
                        // one just beneath the top when it brings several
                        // (an app brought back pages deep, a page pushed
                        // before the app's window was first drawn).
                        let base = if (1..depth).contains(&transition.over) { transition.over } else { depth - 1 };
                        let backdrop = transition.backdrop.take().map(|px| (px, None));
                        let under = match backdrop.or_else(|| kept(base)) {
                            Some(under) => under,
                            None => picture(g, self.stack[base - 1].0.as_mut(), base == 1, false),
                        };
                        [under, picture(g, self.stack[depth - 1].0.as_mut(), false, true)]
                    }
                });
                transition.rebase();
                self.last_depth = usize::MAX;
                g.focus_static = false;
                g.canvas.px.copy_from_slice(&self.background);
                transition.made = PICTURES_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            } else if !transition.ghost && g.revealing() {
                // The page on its way in (or turned back on its way) is
                // still rising: draw it again.
                g.focus_static = true;
                g.pager = None;
                let screen = match &mut transition.leaving {
                    Some(leaving) => leaving.as_mut(),
                    None => self.stack[depth - 1].0.as_mut(),
                };
                Self::draw_screen(g, screen, &self.robot, false);
                if !screen.full_screen() {
                    rise_in(g, &self.background, &mut self.rise_scratch);
                }
                let dots = g.pager.take().map(|(pages, current, _)| (pages, current));
                g.pager_at = None;
                if let Some([_, over]) = &mut transition.pictures {
                    over.0.copy_from_slice(&g.canvas.px);
                    over.1 = dots;
                }
                transition.made = PICTURES_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                g.focus_static = false;
                g.canvas.px.copy_from_slice(&self.background);
            }
            let (p, path) = (transition.progress.value(), transition.path);
            let (under_dots, over_dots) = match &transition.pictures {
                Some([(_, under), (_, over)]) => (*under, *over),
                None => unreachable!("taken above"),
            };
            // Motion still playing beneath or within it is drawn afresh each
            // frame, as one picture each: the GPU takes the pictures up again.
            if transition.beneath.is_some() || transition.within.is_some() {
                transition.made = PICTURES_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            }
            let made = transition.made;
            let (under, over) = transition.layers(&self.background, g.tile_radius(), t.ground).expect("taken above");
            match path {
                Path::Slide => {
                    // The one underneath drifts a third as far as the one on top travels.
                    let (under_x, over_x) = compose_slide(&mut g.canvas.px, under, over, p);
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
                        Some(gpu) => gpu.grow(&mut g.canvas, under, over, made, window, corner, t.ground, Some(&morph)),
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

        // Status at the top right: links, battery, the time. Like the title,
        // it fades out under an island grown over it, and back.
        let ty = g.ty;
        let bar_cy = 4.0 + g.island_h() * 0.5;
        let strip = (4.0 + g.island_h() + 3.0) as usize * W as usize;
        let status_alpha = self.status_shown.value().clamp(0.0, 1.0);
        let under_status = (status_alpha < 0.996).then(|| g.canvas.px[..strip].to_vec());
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
        if let Some(under) = &under_status {
            bezel::simd::mix_into(&mut g.canvas.px[..strip], Some(under), ((1.0 - status_alpha) * 256.0) as u32);
        }
        // Where the status begins: the island growing past it, it gives way.
        self.status_left = x;
        // With Spotlight up, the island and the page dots go through the
        // glass pass with its bar, over this frame held still (`draw_spotlight`);
        // so with a sheet up (`draw_sheets`).
        if self.spotlight.visible() || self.sheets.visible() {
            if g.take_motion_started() {
                self.moving = true;
            }
            return;
        }
        self.draw_chrome(began, None, 0.0);
    }

    /// Where the island is just now: as wide as its words, however far its
    /// spring has taken it.
    fn island_rect(&self) -> Rect {
        let island_h = self.gfx.island_h();
        let iw = self.island_w.value().max(island_h);
        Rect::new((W - iw) * 0.5, 4.0, iw, island_h)
    }

    /// The glass over the screen: the island, the page dots' platter, and
    /// Spotlight's bar when it is up (`bar`, in the island's group, so it
    /// grows out of it). `hide`: how far Spotlight has come, as the island's
    /// words and the page dots give way to it.
    fn draw_chrome(&mut self, began: std::time::Instant, bar: Option<GlassShape>, hide: f32) {
        let g = &mut self.gfx;
        let t = g.tokens;
        let shown = (1.0 - 2.0 * hide).clamp(0.0, 1.0);
        // The island: link status at rest, growing to carry a message.
        let linked = self.robot.linked();
        let (label, icon) = match (&self.message, self.settings.island) {
            (Some((text, _)), _) => (text.clone(), true),
            (None, IslandRest::Clock) => (self.status.clock.clone(), false),
            (None, IslandRest::Battery) => (self.status.battery.map_or_else(|| "battery –".to_owned(), |p| format!("battery {p}%")), false),
            (None, IslandRest::Robot) if linked => (self.robot.string("Robot/Identity/Name").unwrap_or_else(|| "robot".into()).to_lowercase(), false),
            (None, IslandRest::Robot) => (self.robot.status(), false),
        };
        // Push-to-talk listening, or an answer being said: the island says
        // so, its meter live beside the words.
        let talk = self.voice.island();
        let (label, icon) = talk.as_ref().map_or((label, icon), |talk| (talk.words.clone(), true));
        // Live activities: with nothing being said, the most important
        // carries the island, compact, and a second sits beside it as a
        // bubble; one that has just changed opens the island to its card for
        // a moment (liveactivity.rs).
        let quiet = self.message.is_none() && talk.is_none() && hide <= 0.0;
        let activity = if quiet { self.activities.list().first().cloned() } else { None };
        let opened = if quiet { self.activities.opened().cloned() } else { None };
        let second = if activity.is_some() { self.activities.list().get(1).cloned() } else { None };
        let open = if opened.is_some() { self.activities.open.value().clamp(0.0, 1.08) } else { 0.0 };
        let look = self.activities.look(g, self.settings.calm);
        let mut bars_seen = false;
        let label = activity.as_ref().map_or(label, |a| a.short.clone());
        let caption = g.ty.caption;
        let island_h = g.island_h();
        // A message may take most of the width; status stays out of the title's way.
        let room = if icon { 250.0 } else { 96.0 };
        let label = g.text.fit(&label, &caption, room);
        // At rest, notifications not yet seen: the dot in the signal colour,
        // and how many at the island's end.
        let unread = if icon || activity.is_some() { 0 } else { self.notes.unread() };
        let count = (unread > 0).then(|| if unread > 99 { "99+".to_owned() } else { unread.to_string() });
        let count_w = count.as_ref().map_or(0.0, |c| g.measure(c, caption) + 8.0);
        // Compact, the island does not reach the time and the battery: it is
        // as wide as leaves them clear, its words what matters most of them
        // (it opens to its card for the rest). The bubble beside it counts,
        // on both sides. Something urgent (queue now, a timer's last minute)
        // takes the room for the while it is.
        let bubble_room = second.as_ref().map_or(0.0, |_| island_h + 6.0);
        let compact_most = (2.0 * (self.status_left - 2.0) - W - 2.0 * bubble_room).max(70.0);
        let want = match &activity {
            Some(_) if self.activities.open.target() > 0.5 && opened.is_some() => W - 2.0 * PAD,
            Some(a) if a.urgent => crate::liveactivity::compact_w(g, a),
            Some(a) => crate::liveactivity::compact_w(g, a).min(compact_most),
            None => g.measure(&label, caption) + 34.0 + count_w,
        };
        let want = want + talk.as_ref().map_or(0.0, |_| crate::voice::METER_ROOM);
        if !icon && activity.is_none() {
            self.island_rest = want;
        }
        // The bubble beside it takes room too, on both sides (it is centred).
        let bubble = second.as_ref().map_or(0.0, |_| island_h + 6.0) * (1.0 - open).clamp(0.0, 1.0);
        // The title and the status give way to the island where they would
        // meet: out as soon as it is growing over them, back once it has
        // shrunk clear. (Two pixels' grace for the island's spring running a
        // touch past the width it grows to.)
        let title_w = match self.stack.last() {
            Some((top, _)) if !top.full_screen() => {
                let title = g.text.fit(&shown_title(g.theme, top.title()), &g.ty.title, g.title_room);
                g.measure(&title, g.ty.title)
            }
            _ => 0.0,
        };
        let reach = (want + 2.0 * bubble).max(self.island_w.value() - 2.0).max(island_h);
        let beside = (W - reach) * 0.5 - PAD - 8.0;
        let clear_of = (W + reach) * 0.5 + 2.0;
        for (fade, fits) in [(&mut self.title_shown, title_w <= beside), (&mut self.status_shown, self.status_left >= clear_of)] {
            let to = if fits { 1.0 } else { 0.0 };
            if fade.target() != to {
                fade.set(to);
                self.moving = true;
            }
            if self.settings.calm {
                fade.jump(to);
            }
        }
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
        let open_h = self.activities.open_h(g);
        let ih = island_h + (open_h - island_h) * open;
        let island = Rect::new((W - iw) * 0.5, 4.0, iw, ih);
        let island_radius = island_h * 0.5 + (crate::liveactivity::OPEN_RADIUS - island_h * 0.5) * open.min(1.0);
        self.places.island = Some(island);
        let bubble_rect = Rect::new(island.right() + 6.0, 4.0, island_h, island_h);
        // Every glass shape goes through one pass; each family is its own
        // group. Spotlight's bar goes first in the island's: where the two
        // meet, it is the bar's; and the island's glass gives way as the
        // bar's materializes, so at rest the bar is the island, grown.
        let island_glass = bar.map_or(1.0, |b| 1.0 - b.strength);
        let mut shapes: Vec<GlassShape> = bar.into_iter().collect();
        shapes.push(GlassShape { strength: island_glass, ..GlassShape::new(island, island_radius, 0) });
        // The bubble: glass of its own group, so it stands apart from the
        // island rather than melting into it; it materializes as the island
        // closes and goes as it opens.
        if second.is_some() && open < 0.98 {
            shapes.push(GlassShape { strength: island_glass * (1.0 - open), ..GlassShape::new(bubble_rect, island_h * 0.5, 3) });
        }
        // Too many stops for a dot each (Flow's apps): a short track, with
        // the place marked on it as it moves.
        let along = g.pager_at.take();
        let pager = g.pager.map(|(pages, current, ox)| {
            let w = if pages > PAGER_DOTS { SCRUB_W } else { pages as f32 * 9.0 + 7.0 };
            (Rect::new((W - w) * 0.5 + ox, H - 13.0, w, 10.0), pages, current)
        });
        if let Some((platter, ..)) = pager {
            // Under Spotlight's bar the page dots dematerialize, as glass leaves.
            shapes.push(GlassShape { strength: 1.0 - hide, ..GlassShape::new(platter, 5.0, 1) });
        }
        self.places.pager = pager.map(|(platter, ..)| platter);
        let before_glass = std::time::Instant::now();
        g.glass(&shapes);
        self.spent.0 += before_glass - began;
        self.spent.1 += before_glass.elapsed();
        if let Some((platter, pages, current)) = pager.filter(|(_, pages, _)| *pages > PAGER_DOTS) {
            // Faint dots along the track, and the place: Bezel's current dot,
            // a short ink capsule, sliding along it.
            let (x0, x1) = (platter.x + 8.0, platter.right() - 8.0);
            for i in 0..7 {
                g.canvas.circle(x0 + (x1 - x0) * i as f32 / 6.0, platter.cy(), 1.5, t.faint, 1.0 - hide);
            }
            let at = along.unwrap_or(current as f32 / (pages - 1).max(1) as f32).clamp(0.0, 1.0);
            let thumb = Rect::new(x0 - 5.0 + (x1 - x0) * at, platter.cy() - 2.0, 10.0, 4.0);
            g.canvas.rrect(thumb, 2.0, t.ink, 1.0 - hide);
        } else if let Some((platter, pages, current)) = pager {
            for i in 0..pages {
                let color = if i == current { t.ink } else { t.faint };
                g.canvas.circle(platter.x + 8.0 + i as f32 * 9.0, platter.cy(), 2.0, color, 1.0 - hide);
            }
        }

        // What sits on glass is not glass. New words fade in once the island
        // is past halfway (55%) to its new width.
        let travel = want - self.island_from;
        let grown = if travel.abs() < 1.0 { 1.0 } else { ((self.island_w.value() - self.island_from) / travel).clamp(0.0, 1.0) };
        let words = ((grown - 0.55) / 0.45).clamp(0.0, 1.0);
        g.canvas.set_clip(island.inset(2.0));
        // Under a sheet grown out of the island (Spotlight, the clipboard,
        // quick capture), push-to-talk's icon and meter are not drawn: the
        // sheet says it listens itself.
        if let Some(talk) = talk.as_ref().filter(|_| hide <= 0.0) {
            crate::voice::draw_island(g, island, talk, &label, words * shown);
        } else if let Some(a) = &activity {
            // Open: the card, its words in once the island is most of the
            // way there; closed: compact, its words in as the island settles.
            if let (Some(o), true) = (&opened, open > 0.02) {
                let card = ((open - 0.5) / 0.4).clamp(0.0, 1.0);
                crate::liveactivity::draw_card_look(g, island.inset(4.0), o, &look, card * shown);
            }
            let compact = (1.0 - open / 0.45).clamp(0.0, 1.0);
            if compact > 0.0 {
                let compact_rect = Rect::new(island.x, island.y, island.w, island_h);
                crate::liveactivity::draw_compact(g, compact_rect, a, &look, words * compact * shown);
                // Still on StandBy (a night on the charger draws only as the
                // face changes) and with the battery saver: not 60 frames a
                // second for the bars.
                bars_seen = a.kind == crate::liveactivity::Kind::Music && words * compact * shown > 0.0 && !self.faces.showing() && !self.settings.battery.saver;
            }
        } else {
            let dot = if icon || unread > 0 { t.signal } else if linked { t.ice } else { t.faint };
            g.canvas.circle(island.x + 12.0, island.cy(), 3.0, dot, shown);
            g.text_alpha(island.x + 22.0, island.cy(), &label, caption, t.ink, words * shown);
            if let Some(count) = &count {
                let w = g.measure(count, caption);
                g.text_alpha(island.right() - 12.0 - w, island.cy(), count, caption, t.signal, words * shown);
            }
        }
        g.canvas.clear_clip();
        self.activities.bars_seen |= bars_seen;
        if let (Some(b), true) = (&second, open < 0.5) {
            crate::liveactivity::draw_bubble(g, bubble_rect, b, shown * (1.0 - open * 2.0));
        }
        if g.take_motion_started() {
            self.moving = true;
        }
    }

    fn draw_screen(g: &mut Gfx, screen: &mut dyn Screen, robot: &Robot, _home: bool) {
        if !screen.full_screen() {
            draw_title(g, screen.title());
        }
        // What this screen draws along the foot, and its lists, start afresh;
        // and what Copy would take from it.
        g.foot = H;
        g.viewports.clear();
        g.copy_field = None;
        g.copy_row = None;
        screen.draw(g, robot);
        // Page dots asked for sit on their glass along the foot (`draw_chrome`).
        if g.pager.is_some() {
            g.take_foot(H - 16.0);
        }
    }
}

/// A screen's title, on the island's line; it gives way to a wide island.
/// (A screen that paints the whole panel, Flow, draws it again over that.)
pub(crate) fn draw_title(g: &mut Gfx, title: &str) {
    let (t, ty) = (g.tokens, g.ty);
    let bar_cy = 4.0 + g.island_h() * 0.5;
    let title = g.text.fit(&shown_title(g.theme, title), &ty.title, g.title_room.max(0.0));
    g.text_alpha(PAD + 2.0, bar_cy, &title, ty.title, t.ink, g.title_alpha);
}

/// Points along a capsule's outline from the top of its middle, clockwise,
/// `p` of the way round: the hold cue's line.
fn capsule_path(r: Rect, p: f32) -> Vec<(f32, f32)> {
    let rad = r.h * 0.5;
    let straight = (r.w - r.h).max(0.0);
    let arc = std::f32::consts::PI * rad;
    let total = 2.0 * straight + 2.0 * arc;
    let want = (p.clamp(0.0, 1.0) * total).max(0.5);
    let (cy, left_c, right_c) = (r.cy(), r.x + rad, r.right() - rad);
    let at = |d: f32| -> (f32, f32) {
        // Top edge, middle to the right; the right end; the bottom, right to
        // left; the left end; the top, left to the middle.
        let half = straight * 0.5;
        if d <= half {
            return (r.cx() + d, r.y);
        }
        let d = d - half;
        if d <= arc {
            let a = d / rad;
            return (right_c + rad * a.sin(), cy - rad * a.cos());
        }
        let d = d - arc;
        if d <= straight {
            return (right_c - d, r.bottom());
        }
        let d = d - straight;
        if d <= arc {
            let a = d / rad;
            return (left_c - rad * a.sin(), cy + rad * a.cos());
        }
        let d = d - arc;
        (left_c + d, r.y)
    };
    let steps = ((want / 2.0).ceil() as usize).max(2);
    (0..=steps).map(|i| at(want * i as f32 / steps as f32)).collect()
}

/// A screen's title as the look writes it: Bezel's headings are lowercase,
/// as its panels' are.
fn shown_title(theme: crate::look::Theme, title: &str) -> String {
    match theme {
        crate::look::Theme::Bezel => title.to_lowercase(),
        crate::look::Theme::Catalyst => title.to_owned(),
    }
}

impl Shell {
    /// Push-to-talk and answers said aloud, in time: the talk key held
    /// anywhere listens; let go, what was said goes where it was meant to.
    /// True while any of it is under way (the screen stays awake for it).
    fn tick_voice(&mut self, dt: f32) -> bool {
        use crate::voice::{Dictation, Event, Target};
        let down = crate::voice::talk_key_down();
        let awake = self.standby.state() == crate::standby::State::Awake;
        // Where it would go, looked at only while the key is down. Not over
        // the start, the tour, the control centre or the recent apps.
        // Never under the lock screen or StandBy: the microphone and the
        // assistant are past the passcode.
        let in_the_way = self.intro.is_some() || self.tour.is_some() || self.switcher.is_some() || self.control.visible() || self.lock.covering() || self.faces.showing() || self.sheets.panel.is_open();
        let here = match self.stack.last() {
            // Quick capture's sheet takes what is said, as a note's page does.
            _ if down && !in_the_way && self.sheets.capture.is_open() => Some(Target { kind: Dictation::Text, depth: usize::MAX, title: crate::capture::TITLE.to_owned() }),
            Some((top, _)) if down && !in_the_way && !top.full_screen() => {
                Some(Target { kind: top.dictation().unwrap_or(Dictation::Question), depth: self.stack.len(), title: top.title().to_owned() })
            }
            _ => None,
        };
        self.voice.calm = self.settings.calm;
        match self.voice.tick(dt, down, awake, here) {
            Some(Event::Woke) => {
                self.standby.key();
                self.last_key = std::time::Instant::now();
            }
            Some(Event::Say(text)) => self.say(text),
            Some(Event::Heard(text, target)) => self.heard(&text, &target),
            None => {}
        }
        // Someone holding the key and talking: no dimming meanwhile. (An
        // answer being said keeps nothing awake: put down, it may sleep.)
        if self.voice.listening() {
            self.standby.key();
        }
        let busy = self.voice.active();
        if busy {
            self.dirty = true;
        }
        busy
    }

    /// What was said, written down: typed into the page it was said over
    /// (if that is still in front), or asked of the assistant, which grows
    /// out of the island with it (a kept one comes back, conversation and
    /// all).
    fn heard(&mut self, text: &str, target: &crate::voice::Target) {
        use crate::voice::Dictation;
        // Locked while it was being written down (Tab held, the screen
        // gone off): what was said goes nowhere.
        if self.lock.covering() || self.faces.showing() {
            return;
        }
        if target.depth == usize::MAX && target.title == crate::capture::TITLE {
            return self.heard_for_capture(text);
        }
        let takes = self.stack.last().and_then(|(top, _)| top.dictation());
        let same_page = self.stack.len() == target.depth && self.stack.last().is_some_and(|(top, _)| top.title() == target.title);
        let at = match (takes, target.kind) {
            (Some(Dictation::Text), Dictation::Text) if same_page => Some(self.stack.len() - 1),
            (Some(Dictation::Question), Dictation::Question) => Some(self.stack.len() - 1),
            // The page it was for has gone: what was said is not lost.
            (_, Dictation::Text) => return self.say(format!("heard: {text}")),
            _ => {
                let from = self.places.island.unwrap_or_else(|| Rect::new(W * 0.5 - 30.0, 4.0, 60.0, self.gfx.island_h()));
                self.open(Box::new(crate::ai::Assistant::new()), Path::Grow(from));
                self.stack.iter().rposition(|(s, _)| s.dictation() == Some(Dictation::Question))
            }
        };
        let Some(at) = at else { return };
        let mut say = None;
        let at_rest = self.transition.is_none();
        let nav = self.stack[at].0.dictated(text, &mut Cx { robot: &mut self.robot, settings: &mut self.settings, say: &mut say, at_rest });
        if let Some(text) = say {
            self.say(text);
        }
        if at + 1 == self.stack.len() {
            self.follow(nav);
        }
        self.moving = true;
        self.dirty = true;
    }

    /// What the island says for push-to-talk, else its message (the tests).
    #[cfg(test)]
    pub(crate) fn island_words(&self) -> Option<String> {
        self.voice.island().map(|i| i.words).or_else(|| self.message.as_ref().map(|(m, _)| m.clone()))
    }

    /// The island set outright, push-to-talk's or a message (or neither),
    /// for the gallery to check each over each screen.
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub fn stage_island(&mut self, talk: Option<crate::voice::Island>, message: Option<&str>) {
        self.voice.staged = talk;
        self.message = message.map(|m| (m.to_owned(), f32::INFINITY));
        self.moving = true;
        self.dirty = true;
    }

    /// How much of the title shows just now, 0 to 1.
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub fn title_alpha(&self) -> f32 {
        self.title_shown.value()
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
    scale_into(&mut canvas.px, (canvas.w, canvas.h), src, window, radius, edge, morph);
}

/// The same into a bare picture `w` × `h`: what plays beneath or within a
/// transition is drawn into one.
fn scale_into(out: &mut [u32], (w, h): (i32, i32), src: &[u32], window: Rect, radius: f32, edge: Color, morph: Option<&Morph>) {
    if window.w < 1.0 || window.h < 1.0 {
        return;
    }
    // Only a tile that is really there, and partly showing, is drawn.
    let morph = morph.filter(|m| m.screen < 1.0 && m.tile.w >= 1.0 && m.tile.h >= 1.0 && m.under.len() == src.len());
    // The whole panel: a straight copy.
    if morph.is_none() && window.w >= w as f32 - 0.5 && window.h >= h as f32 - 0.5 {
        out.copy_from_slice(src);
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
                out[at] = blend(out[at], Color::BLACK, 0.22 * fall * fall);
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
                out[at] = px;
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
            out[at] = mix_px(out[at], px, cover);
        }
    }
}

// ------------------------------------- recent apps, screenshots, standby ---

impl Shell {
    /// Where the app in front starts on the stack: its first page kept among
    /// the recent apps (search, the shortcut list and the like are not).
    /// None on Home.
    fn app_start(&self) -> Option<usize> {
        (1..self.stack.len()).find(|&i| self.stack[i].0.keep_in_recents())
    }

    /// A screen gone: an app gone back to Home is kept among the recent apps,
    /// with the picture it last showed; any other page is done with.
    fn left(&mut self, mut screen: Box<dyn Screen>, path: Path, picture: Option<Vec<u32>>) {
        screen.set_aside();
        if self.stack.len() == 1 && screen.keep_in_recents() {
            self.recents.remember(vec![(screen, path)], picture.as_deref());
        }
    }

    /// The screen in front as it last showed at rest, or as it arrives if it
    /// is still on its way in; drawn now if it has been neither.
    fn top_picture(&mut self) -> Picture {
        let depth = self.stack.len();
        if self.last_depth == depth {
            return self.last_screen.clone();
        }
        if let Some(Transition { pictures: Some([_, over]), .. }) = self.transition.as_ref().filter(|t| t.arriving()) {
            return over.clone();
        }
        Self::picture_of(&mut self.gfx, &self.background, &self.robot, self.stack[depth - 1].0.as_mut(), depth == 1)
    }

    /// The panel as it last showed fades out over whatever is drawn next,
    /// where what is drawn next could not carry on from it. True if it does
    /// (not in the calm version).
    fn fade_out_shown(&mut self) -> bool {
        if self.settings.calm {
            return false;
        }
        let mut fade = Animated::new(1.0, role::smooth()).with_rest(0.004);
        fade.set(0.0);
        self.curtain = Some((self.gfx.canvas.px.clone(), fade));
        self.moving = true;
        true
    }

    /// A screen just taken off the top, as it last showed at rest there, or
    /// drawn now if it never did.
    fn leaving_picture(&mut self, screen: &mut dyn Screen) -> Picture {
        match self.last_depth == self.stack.len() + 1 {
            true => self.last_screen.clone(),
            false => Self::picture_of(&mut self.gfx, &self.background, &self.robot, screen, false),
        }
    }

    /// `screen` drawn now as it rests, over the wallpaper, with the page
    /// dots it asks for; the panel's picture is left as it was.
    fn picture_of(g: &mut Gfx, background: &[u32], robot: &Robot, screen: &mut dyn Screen, home: bool) -> Picture {
        let shown = g.canvas.px.clone();
        if background.len() == shown.len() {
            g.canvas.px.copy_from_slice(background);
        }
        (g.focus_static, g.pager) = (true, None);
        Self::draw_screen(g, screen, robot, home);
        let dots = g.pager.take().map(|(pages, current, _)| (pages, current));
        g.pager_at = None;
        g.focus_static = false;
        (std::mem::replace(&mut g.canvas.px, shown), dots)
    }

    /// The recent apps. The card of the app in front shows it as it was last
    /// drawn, or `under`, the picture the control centre came down over.
    pub fn open_recents(&mut self, under: Option<Vec<u32>>) {
        use crate::recents::{icon_for, thumbnail, Card, Switcher};
        // A screen on its way out is let go now (an app among the recent
        // apps, so it has its card), and its motion plays on behind the
        // cards as they come; one on its way in goes on arriving.
        if self.transition.as_ref().is_some_and(|t| t.leaving.is_some()) {
            self.transition = self.interrupt().map(|t| *t);
        }
        let current = self.app_start().map(|start| {
            let title = self.stack[start].0.title().to_owned();
            let picture = under.unwrap_or_else(|| self.top_picture().0);
            Card { icon: icon_for(&title), title, thumb: thumbnail(&picture), current: true }
        });
        let kept = self.recents.iter().map(|r| Card { title: r.title().to_owned(), icon: icon_for(r.title()), thumb: r.thumb.clone(), current: false });
        let mut switcher = Switcher::new(current.into_iter().chain(kept).collect(), self.settings.calm);
        // Brought up again as they went: from where they were.
        if let Some(old) = &self.switcher {
            switcher.carry_on_from(old);
        }
        self.switcher = Some(switcher);
        self.switcher_under = None;
        self.moving = true;
        self.dirty = true;
    }

    /// Whether the recent apps are showing (and not on their way out): for
    /// the tests and the galleries.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn recents_open(&self) -> bool {
        self.switcher.as_ref().is_some_and(|s| !s.leaving())
    }

    fn switcher_key(&mut self, key: Key) {
        use crate::recents::Action;
        self.moving = true;
        let Some(switcher) = &mut self.switcher else { return };
        match switcher.key(key) {
            Action::Stay => {}
            Action::Close => switcher.close(),
            Action::Open(at) => self.switch_to(at),
            Action::Dismiss(at) => self.let_go(at),
        }
    }

    /// The app on card `at` to the front, growing out of its card; the app
    /// that was in front joins the recent apps.
    fn switch_to(&mut self, at: usize) {
        let Some(mut switcher) = self.switcher.take() else { return };
        let current = switcher.cards.first().is_some_and(|c| c.current);
        if current && at == 0 {
            // The app in front already: the cards go, and there it is.
            switcher.close();
            self.switcher = Some(switcher);
            return;
        }
        let Some(recent) = self.recents.take(at - current as usize) else { return };
        let from = switcher.card_rect(at);
        // It grows out of its card, over the cards as they showed.
        let backdrop = self.gfx.canvas.px.clone();
        if let Some(start) = self.app_start() {
            let mut pages = self.stack.split_off(start);
            for (page, _) in &mut pages {
                page.set_aside();
            }
            let thumb = switcher.cards.swap_remove(0).thumb;
            self.recents.keep(pages, thumb);
        }
        self.stack.truncate(1);
        self.switcher_under = None;
        self.bring_back(recent, from, Some(backdrop));
    }

    /// Let the app on card `at` go. The app in front, let go, closes for good
    /// and Home shows behind the cards.
    fn let_go(&mut self, at: usize) {
        let Some(switcher) = &mut self.switcher else { return };
        let current = switcher.cards.first().is_some_and(|c| c.current);
        switcher.remove(at);
        if current && at == 0 {
            self.stack.truncate(1);
            self.transition = None;
            self.gfx.focus_reset();
            if let Some((home, _)) = self.stack.last_mut() {
                home.resume();
            }
            self.switcher_under = None;
        } else {
            self.recents.forget(at - current as usize);
        }
    }

    /// An app kept among the recent apps, back in front with its pages as
    /// they were, growing out of `from` over `backdrop` (or the screen beneath).
    fn bring_back(&mut self, recent: crate::recents::Recent, from: Rect, backdrop: Option<Vec<u32>>) {
        self.gfx.focus_reset();
        self.gfx.reveal_stop();
        if let Some(words) = recent.stack.last().and_then(|(s, _)| s.greeting()) {
            self.say(words);
        }
        if self.last_depth > self.stack.len() {
            self.last_depth = usize::MAX;
        }
        let over = self.stack.len();
        self.stack.extend(recent.stack);
        if let Some((top, _)) = self.stack.last_mut() {
            top.resume();
        }
        self.moving = true;
        self.dirty = true;
        if self.settings.calm {
            self.transition = None;
            return;
        }
        let mut transition = Transition::new(Path::Grow(from), 0.0, 1.0, None, backdrop);
        transition.over = over;
        self.transition = Some(transition);
    }

    /// Save `picture`, the panel as it showed, as a screenshot: a flash says
    /// it was taken, and the island once it is saved.
    fn screenshot(&mut self, picture: Vec<u32>) {
        self.saving = Some(crate::screenshot::save(picture));
        if !self.settings.calm {
            let mut flash = Animated::new(1.0, role::smooth()).with_rest(0.004);
            flash.set(0.0);
            self.flash = Some(flash);
        }
        self.moving = true;
        self.dirty = true;
    }

    /// The switcher, a screenshot's flash and its saving, and standby, in time.
    fn tick_recents_and_standby(&mut self, dt: f32, motion_dt: f32) -> bool {
        let mut moving = false;
        if let Some(switcher) = &mut self.switcher {
            moving |= switcher.tick(motion_dt);
            if switcher.gone() {
                self.switcher = None;
                self.switcher_under = None;
                self.dirty = true;
            }
        }
        if let Some(flash) = &mut self.flash {
            moving |= flash.tick(motion_dt);
            if flash.value() <= 0.004 {
                self.flash = None;
                self.dirty = true;
            }
        }
        if let Some(saving) = &self.saving {
            match saving.try_recv() {
                Ok(done) => {
                    self.saving = None;
                    self.say(match done {
                        Ok(_) => "screenshot saved".to_owned(),
                        Err(why) => why,
                    });
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.saving = None,
            }
        }
        moving |= self.lift_and_lay_down();
        let keep = self.stack.last().is_some_and(|(s, _)| s.keeps_awake()) || self.saving.is_some();
        self.standby.set_faces(self.faces_wanted());
        moving |= self.watch_charger();
        moving |= self.standby.tick(dt, self.settings.screen_off, keep, self.settings.calm);
        moving |= self.tick_lock_and_faces(dt, motion_dt);
        moving
    }

    // ------------------------------------------------ StandBy and the lock ---

    /// On the charger (as the gauge says, or the galleries pretend).
    fn on_charger(&self) -> bool {
        self.charger.unwrap_or_else(|| self.care.reading.as_ref().is_some_and(|r| matches!(r.flow, crate::power::Flow::Charging | crate::power::Flow::Full | crate::power::Flow::Holding)))
    }

    /// Whether StandBy may come where the screen would go off, and how lit:
    /// on the charger with StandBy on, once the handheld is set up.
    fn faces_wanted(&self) -> Option<f32> {
        let set_up = !self.stack.iter().any(|(s, _)| s.title() == "Setup");
        (self.settings.lock.standby && self.on_charger() && set_up).then(|| crate::faces::Faces::light_level(&self.settings.lock))
    }

    /// The galleries' and tests' charger.
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub fn pretend_charger(&mut self, on: Option<bool>) {
        self.charger = on;
        // The lock screen reads the charge afresh.
        self.lock_info = None;
    }

    /// The galleries' and tests' live activities, as if just read (and
    /// not read again).
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub fn pretend_activities(&mut self, list: Vec<crate::liveactivity::Activity>) {
        self.activities.set(list, self.settings.calm);
        self.dirty = true;
    }

    /// The galleries' hold cue, part way.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn pretend_hold(&mut self, p: f32) {
        self.hold_cue.jump(p);
    }

    /// The island closed again at once (for the galleries' still shots of it compact).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn close_island_for_gallery(&mut self) {
        self.activities.close_now();
    }

    /// The island opened on an activity, read afresh (for the galleries).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn open_island_for_gallery(&mut self, kind: crate::liveactivity::Kind) {
        self.activities.refresh();
        let _ = self.tick(1.0 / 60.0);
        self.activities.show(kind, self.settings.calm);
    }

    /// Read the activities afresh at the next frame (for the galleries).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn refresh_activities_for_gallery(&mut self) {
        self.activities.refresh();
    }

    /// The lock screen, as Tab held brings it (for the galleries).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn lock_as_if_held(&mut self) {
        self.lock_by_hand();
    }

    /// Set down on the charger with the screen dimmed or off (or in deep
    /// standby): StandBy comes up at once, as a phone's does. True if it did.
    fn watch_charger(&mut self) -> bool {
        let on = self.on_charger();
        let plugged = on && !self.charger_was;
        if self.charger_was && !on {
            // Unplugged: deep standby's wait starts now, not from hours ago.
            self.standby.unplugged();
        }
        self.charger_was = on;
        if plugged && self.faces_wanted().is_some() && self.standby.to_faces() {
            self.dirty = true;
            return true;
        }
        false
    }

    /// The gauge's reading as StandBy and the lock screen show it: on the
    /// galleries' pretend charger, the demo battery charging.
    fn shown_reading(&self) -> Option<crate::power::Reading> {
        let reading = self.care.reading.clone();
        match (self.charger, reading) {
            (Some(true), Some(r)) if r.flow.on_battery() => Some(crate::power::Reading { flow: crate::power::Flow::Charging, milliamps: Some(-820.0), minutes_left: None, minutes_to_full: Some(42), ..r }),
            (_, reading) => reading,
        }
    }

    /// StandBy is showing.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn faces_showing(&self) -> bool {
        self.faces.showing()
    }


    /// A passcode typed is being checked (for the galleries).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn checking_passcode(&self) -> bool {
        self.lock.checking()
    }

    /// Locked now (the screen gone off, or StandBy come, a passcode set):
    /// whatever was over the screen in front is put away, so the lock is
    /// what a wake comes back to.
    fn lock_now(&mut self) {
        self.lock.engage();
        self.notes.mark();
        if self.control.is_open() || self.control.visible() {
            self.control.set_open(false, &self.settings, &self.notes);
            self.control.shown.jump(0.0);
            self.control_under = None;
            self.control_glass = None;
        }
        self.switcher = None;
        self.switcher_under = None;
        self.help_hold = None;
        self.message = None;
        // The clipboard's history and quick capture, and a secret copied (shell/clip.rs).
        self.sheets_at_the_lock();
        self.dirty = true;
    }

    /// StandBy gives way (a key, the charger gone): to the lock screen if it
    /// is locked; otherwise what was said meanwhile is said now.
    fn leave_faces(&mut self) {
        if !self.faces.showing() {
            return;
        }
        self.faces.hide(self.settings.calm);
        if !self.lock.engaged() {
            self.say_kept();
        }
        self.moving = true;
        self.dirty = true;
    }

    /// The notifications that came while it was locked (or StandBy
    /// showed), said in the island: the newest, with how many there were
    /// (with Quiet on, only those that break through). Else how long deep
    /// standby lasted, if it did.
    fn say_kept(&mut self) {
        let after = self.after_unlock.take();
        let quiet = self.notes.quiet();
        let kept: Vec<&crate::notify::Notification> = self.notes.since_mark().into_iter().filter(|n| !quiet || n.urgent).collect();
        let words = match kept.as_slice() {
            [] => after,
            [one] => Some(one.said()),
            [newest, rest @ ..] => Some(format!("{} new · {}", rest.len() + 1, newest.title)),
        };
        if let Some(words) = words {
            // Said as the island says anything, but not kept again.
            self.message = Some((words, MESSAGE_SECONDS));
        }
    }

    /// Time for StandBy and the lock: each follows where standby stands.
    fn tick_lock_and_faces(&mut self, dt: f32, motion_dt: f32) -> bool {
        use crate::standby::State;
        let calm = self.settings.calm;
        let state = self.standby.state();
        // The screen has gone off, or StandBy has come: locked, if a passcode is set.
        if matches!(state, State::Off | State::Faces) && !self.lock.engaged() && self.lock.armed() {
            self.lock_now();
        }
        // The vault locks with the screen off, StandBy or the lock screen
        // (with a passcode or not), and after two idle minutes.
        crate::vault::watch(matches!(state, State::Off | State::Faces) || self.lock.covering());
        if state == State::Faces && !self.faces.showing() {
            self.faces.show(&self.settings.lock, calm);
            self.notes.mark();
            self.moving = true;
        } else if state != State::Faces && self.faces.showing() {
            self.leave_faces();
        }
        let reading = self.shown_reading();
        let feed = crate::faces::Feed { reading: reading.as_ref(), h24: self.settings.clock24, notices: self.notes.unread(), generation: self.robot.schedule.generation(), activities: self.activities.list(), opened: self.activities.opened(), open: self.activities.open.value(), level: self.activities.level.value() };
        let (faces_moving, faces_changed) = self.faces.tick(dt, motion_dt, &feed, &self.settings.lock, calm);
        // Woken onto the lock screen: its parts rise in again.
        let lit = !self.standby.asleep();
        if lit && !self.lit_was {
            self.lock.woke(calm);
        }
        self.lit_was = lit;
        // The lock screen's robot stands still in the battery saver, under
        // StandBy and in the calm version (depth.rs).
        self.lock.hold_still(self.settings.battery.saver || self.faces.visible() || calm);
        let (lock_moving, lock_changed, unlocked) = self.lock.tick(dt, motion_dt, calm, self.settings.clock24);
        // Live activities: read once a second while the screen is lit, not
        // at all while it is off.
        let unix = chrono::Utc::now().timestamp();
        let sources = crate::liveactivity::Sources { robot: &self.robot, update: &self.update, unix };
        // Music in front shows the song itself: no card over it; nor over
        // the Assistant, whose steps show in the conversation (the island
        // keeps its compact line).
        self.activities.quiet = self.stack.last().and_then(|(s, _)| {
            s.shows_activity().or(match s.title() {
                "Music" => Some(crate::liveactivity::Kind::Music),
                "Assistant" => Some(crate::liveactivity::Kind::Assistant),
                _ => None,
            })
        });
        let (acts_moving, acts_changed) = self.activities.tick(dt, motion_dt, lit, calm, &sources);
        // Tab held towards a lock: the cue fills as it is held, and draws
        // back if it is let go first.
        let cue = crate::platform::tab_down_since().map(|at| {
            let held = at.elapsed().as_secs_f32();
            let (after, hold) = (crate::platform::LOCK_CUE_AFTER.as_secs_f32(), crate::platform::HOLD_TO_LOCK.as_secs_f32());
            ((held - after) / (hold - after)).clamp(0.0, 1.0)
        });
        let cue_moving = match cue {
            Some(p) => {
                self.hold_cue.jump(p);
                true
            }
            None => {
                if self.hold_cue.target() != 0.0 {
                    if calm {
                        self.hold_cue.jump(0.0);
                    } else {
                        self.hold_cue.set(0.0);
                    }
                }
                self.hold_cue.tick(motion_dt)
            }
        };
        if unlocked {
            self.say_kept();
            self.status.age = f32::MAX;
            // A notification picked on the lock screen: where it leads.
            if let Some(id) = self.lock.take_opened() {
                self.open_notification(id);
            }
        }
        if !self.lock.visible() && !self.lock_scratch.is_empty() {
            self.lock_scratch = Vec::new();
        }
        if faces_changed || lock_changed || acts_changed || cue_moving {
            self.dirty = true;
        }
        faces_moving || lock_moving || acts_moving || cue_moving
    }

    /// Tab held: the lock screen comes down over whatever is showing (the
    /// screen woken first, StandBy and deep standby left). With a passcode it
    /// is locked; without, any key lifts it. Locked already: nothing.
    fn lock_by_hand(&mut self) {
        // Dark, in deep standby or StandBy: woken onto the lock screen (with a
        // passcode it is there already, engaged when the screen went off).
        if self.deep.on() {
            self.standby.key();
            self.leave_deep();
        }
        self.standby.wake_now();
        self.leave_faces();
        if self.lock.covering() {
            self.moving = true;
            self.dirty = true;
            return;
        }
        if self.spotlight.visible() {
            self.spotlight.gone_now();
            self.spotlight_under = None;
        }
        // Shown first (coming down), then what was over the screen put away.
        self.lock.show(self.settings.calm);
        self.lock_now();
        self.hold_cue.jump(0.0);
        self.moving = true;
        self.dirty = true;
    }

    /// Music's keys on the lock screen: the player asked (through
    /// `nowplaying`), nothing unlocked; the volume as anywhere.
    fn media_on_lock(&mut self, media: crate::platform::Media) {
        use crate::nowplaying::Control;
        use crate::platform::Media;
        match media {
            Media::PlayPause => crate::nowplaying::control(Control::PlayPause),
            Media::Next => crate::nowplaying::control(Control::Next),
            Media::Previous => crate::nowplaying::control(Control::Previous),
            other => {
                crate::sound::media_key(other);
            }
        }
        self.activities.refresh();
    }

    /// What the lock screen shows besides the time, read every 20 s at most.
    fn lock_info(&mut self) -> crate::lock::Info {
        if self.lock_info.as_ref().is_none_or(|(_, at)| at.elapsed().as_secs() >= 20) {
            let info = crate::lockpages::info_now(&self.robot, self.settings.clock24, self.shown_reading());
            self.lock_info = Some((info, std::time::Instant::now()));
        }
        let mut info = self.lock_info.as_ref().map(|(i, _)| i.clone()).unwrap_or_default();
        info.notes = self.notes.unread_list();
        info.notices = info.notes.len();
        info.activities = self.activities.list().to_vec();
        info.level = self.activities.level.value();
        info.rise = self.lock.entering();
        info
    }

    /// Frames no faster than this (milliseconds), if anything holds them back:
    /// the battery saver, or StandBy (30 a second, on the charger).
    pub fn frame_floor(&self) -> Option<f32> {
        (self.settings.battery.saver || self.faces.showing()).then_some(crate::power::SAVER_FRAME_MS)
    }

    /// Lift to wake, and face down to sleep: the motion sensor watches at
    /// its slowest while either could happen, and standby hears of it.
    /// (The sensor's thread reads it four times a second, and on a lift
    /// wakes the main loop, which with the screen off looks in only every
    /// two seconds otherwise: platform::wake.)
    fn lift_and_lay_down(&mut self) -> bool {
        use crate::standby::State;
        let off = self.standby.state() == State::Off;
        let watch = (self.settings.lift_wake && off) || (self.settings.face_down && !off);
        crate::imu::set_watch(watch && crate::imu::has_motion());
        // Asked every time, so a lift seen while awake is not kept for later.
        let lifted = crate::imu::take_lift();
        if off && lifted && self.settings.lift_wake {
            self.standby.lifted();
            self.dirty = true;
            return true;
        }
        // Face down for two seconds, with no key meanwhile: out.
        let quiet = self.last_key.elapsed() >= std::time::Duration::from_secs_f32(crate::imu::FACE_DOWN_HOLD);
        if !off && self.settings.face_down && quiet && crate::imu::face_down() {
            self.standby.put_down();
            return true;
        }
        false
    }

    /// Where standby stands: awake, dimmed or off (for the tests and galleries).
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn standby_state(&self) -> crate::standby::State {
        self.standby.state()
    }

    /// The battery and the lights, in time: the saver's rules, the
    /// warnings, turning off when empty; and the queue for the next match.
    fn tick_power(&mut self, dt: f32) {
        let asleep = self.standby.asleep();
        let busy = self.update_busy();
        for event in self.care.tick(dt, asleep, &mut self.settings.battery, busy) {
            self.care_event(event);
        }
        // One reading for everything that shows the battery: Home's widget
        // has it, and the island takes its charge as it comes. Drawn again
        // only when what they show of it changes.
        if self.gfx.charge != self.care.reading {
            let shown = |r: &Option<crate::power::Reading>| r.as_ref().map(|r| (r.percent, r.flow, r.minutes_left, r.minutes_to_full));
            if shown(&self.gfx.charge) != shown(&self.care.reading) {
                self.dirty = true;
            }
            self.gfx.charge.clone_from(&self.care.reading);
        }
        if let Some(percent) = self.care.gauge_percent().filter(|p| self.status.battery != Some(*p)) {
            self.status.battery = Some(percent);
            self.dirty = true;
        }
        if self.standby.state() == crate::standby::State::Awake {
            self.lights.seen();
        }
        self.lights.tick(dt);
        // Nobody looking (the screen off, StandBy's faces, locked): the
        // on-device model gives its memory back, after the answer it is
        // writing if it is writing one.
        if matches!(self.standby.state(), crate::standby::State::Off | crate::standby::State::Faces) || self.lock.engaged() {
            crate::local::away();
        }
        self.watch_queue(dt);
        self.watch_reminders(dt);
        // A Clock timer run out, wherever the handheld is: said, and the
        // keyboard's lights blink (with the screen off, Fn until it wakes).
        if let Some(said) = crate::clock::due() {
            self.notify(crate::notify::Notification::new("Clock", "timer", said).urgent().action("builtin:Clock"));
            self.lights.warning();
        }
        // Notifications posted from other threads (a download done, an
        // answer written).
        for n in crate::notify::take_posted() {
            self.notify(n);
        }
        // Sound moving to USB headphones and back: said briefly, as a
        // phone shows it. Not in deep standby, where it is the USB host
        // going quiet and coming back.
        if let Some(said) = crate::sound::take_said() {
            if !self.deep.on() {
                self.say(said);
            }
        }
    }

    /// What the battery's care asks for.
    fn care_event(&mut self, event: crate::power::Event) {
        use crate::power::Event;
        match event {
            // Its warnings are kept (the rest, said and gone).
            Event::Say(text) if text.starts_with("battery at") => {
                self.notify(crate::notify::Notification::said_as("Battery", "battery_alert", &text).tag("battery").action("setting:Battery"));
            }
            Event::Say(text) => self.say(text),
            // In deep standby the keyboard's lights stay dark: the red
            // light's reminder says it instead.
            Event::Warn if self.deep.on() => {}
            Event::Warn => self.lights.warning(),
            Event::Saver => {
                self.apply_settings();
                crate::save_settings(&self.settings);
            }
            Event::Empty => {
                self.standby.wake_now();
                // Over everything: Spotlight gives way at once.
                self.spotlight.gone_now();
                // Locked, the card is beneath the lock: the lock screen says it.
                if self.lock.engaged() {
                    self.lock.alert("battery empty · turning off");
                }
                let card = crate::power::Empty::new(self.settings.calm);
                let card = if crate::power::postponed_floor(&self.care) { card.at_floor() } else { card };
                self.open(Box::new(card), Path::Slide);
            }
        }
    }

    /// An update is being checked for, downloaded or installed.
    fn update_busy(&self) -> bool {
        matches!(self.update.state.as_str(), "checking" | "downloading" | "installing")
    }

    /// In deep standby: the main loop waits a minute at a time for a key.
    pub fn deep_asleep(&self) -> bool {
        self.deep.on()
    }

    /// What going deep now would cut off, if anything.
    fn deep_blocker(&self) -> Option<&'static str> {
        crate::deep::blocker(crate::deep::Busy {
            robot: self.robot.linked() && !self.robot.is_demo(),
            computer: crate::usb::connected() == Some(true),
            ssh: crate::power::ssh_session(),
            update: self.update_busy(),
            timer: timer_running(),
            saving: self.saving.is_some(),
            music: music_playing(),
            downloads: crate::music::downloads_running(),
            charger: self.on_charger(),
        })
    }

    /// What music standby would cut off: what deep standby would, less the
    /// music and a computer on USB (the USB host is left to it), and SSH
    /// only if Wi-Fi would go.
    fn music_blocker(&self) -> Option<&'static str> {
        let wifi = crate::nowplaying::needs_network();
        // Streaming over a cable: the Ethernet adapter is on the USB host
        // that music standby stops.
        if wifi && crate::sys::link_up("eth0") {
            return Some("the music streams over Ethernet");
        }
        crate::deep::blocker(crate::deep::Busy {
            robot: self.robot.linked() && !self.robot.is_demo(),
            computer: false,
            ssh: !wifi && crate::power::ssh_session(),
            update: self.update_busy(),
            timer: timer_running(),
            saving: self.saving.is_some(),
            music: false,
            // Music standby takes Wi-Fi away unless the music needs it.
            downloads: crate::music::downloads_running(),
            charger: false,
        })
    }

    /// Into deep standby: its steps in order (deep.rs says what each is).
    fn enter_deep(&mut self) {
        self.deep.begin(self.care.reading.as_ref());
        for step in crate::deep::STEPS {
            self.deep_step(step, true);
        }
    }

    /// Into music standby: deep standby's steps, the sound (and the radios,
    /// and Wi-Fi while the music streams) left alone.
    fn enter_music_standby(&mut self) {
        self.deep.begin_music(crate::nowplaying::needs_network());
        for step in crate::deep::STEPS {
            self.deep_step(step, true);
        }
    }

    /// Music standby's look, once a minute: Wi-Fi off once what plays next
    /// is all here; once the music has stopped for as long as the screen
    /// takes to go off, deep standby in place if its wait is over, else
    /// back to plain standby (the screen still dark).
    fn look_at_the_music(&mut self, dt: f32) {
        use crate::deep::{MusicNext, Step};
        let linger = if self.settings.screen_off > 0 { self.settings.screen_off as f32 } else { 60.0 };
        let after = self.settings.battery.deep_after;
        let idle = self.standby.idle() + self.deep.dark_for();
        let playing = music_playing();
        let deep_ok = !playing && after > 0 && idle >= after as f32 * 60.0 && self.deep_blocker().is_none();
        match self.deep.music_next(playing, crate::nowplaying::needs_network(), linger, deep_ok, dt) {
            MusicNext::Stay => {}
            // catalyst-power takes only the steps not taken yet.
            MusicNext::WifiOff => self.deep_step(Step::System, true),
            MusicNext::Deepen => {
                let reading = self.care.reading.clone();
                self.deep.deepen(reading.as_ref());
                self.deep_step(Step::System, true);
            }
            MusicNext::Leave => self.leave_quietly(),
        }
    }

    /// Out of music standby with the screen left dark: every step undone,
    /// and the time in it counted as standby's idle time (deep standby's
    /// wait goes on from there).
    fn leave_quietly(&mut self) {
        let reading = self.care.reading.clone();
        let Some(woke) = self.deep.end(reading.as_ref()) else { return };
        for step in crate::deep::waking_first() {
            self.deep_step(step, false);
        }
        self.finish_waking();
        self.standby.add_idle(woke.seconds);
    }

    /// One of deep standby's steps, taken (`deep`) or undone.
    fn deep_step(&mut self, step: crate::deep::Step, deep: bool) {
        use crate::deep::Step;
        match step {
            Step::Pollers if deep => self.robot.pause(),
            Step::Pollers => self.robot.resume(),
            Step::Lights => self.lights.set_dark(deep),
            _ => {}
        }
        crate::deep::take(step, deep, &mut self.deep);
    }

    /// A minute in deep standby: the battery read (low-battery care still
    /// watches over it), the draw noted, the red light's reminder.
    fn tick_deep(&mut self, dt: f32) {
        let events = self.care.tick(dt, true, &mut self.settings.battery, false);
        for event in events {
            self.care_event(event);
        }
        let reading = self.care.reading.clone();
        self.deep.sample(reading.as_ref(), dt);
        if let Some(on) = self.deep.reminder(reading.as_ref()) {
            crate::lights::red_reminder(on);
        }
        // A reminder or the queue come due: awake, to say it. Only the times
        // worked out before it went deep are looked at; nothing is polled.
        let now = crate::reminders::now();
        if [self.reminders.due(), self.queue_due].into_iter().flatten().any(|at| now >= at) {
            self.standby.wake_now();
            self.queue_age = 5.0;
            self.woke_for_alert = true;
        }
        // Set on the charger: StandBy (and out of deep standby, which the
        // shell notices as the screen being lit).
        self.standby.set_faces(self.faces_wanted());
        self.watch_charger();
        if self.deep.music() && self.standby.asleep() {
            self.look_at_the_music(dt);
        }
    }

    /// The main loop's wait for a key in deep standby: a minute, or less
    /// when a reminder or the queue alert comes first. That wait ends in
    /// place of the minute's, so no wakeup is added.
    pub fn deep_wait(&self) -> std::time::Duration {
        let now = crate::reminders::now();
        match [self.reminders.due(), self.queue_due].into_iter().flatten().min() {
            Some(at) => crate::deep::WAIT.min(std::time::Duration::from_secs(at.saturating_sub(now).max(0) as u64)),
            None => crate::deep::WAIT,
        }
    }

    /// Reminders come due: the island holds what is due, the keyboard's
    /// lights blink, and the chime plays (once there is a speaker). With
    /// the screen off or dimmed it wakes, and Reminders opens on them; on a
    /// screen in use, only the island and the lights say it.
    fn watch_reminders(&mut self, dt: f32) {
        // Not under the start animation: what came due while the handheld
        // was off is said once Home is there to see it said.
        if self.intro.is_some() {
            return;
        }
        let woke_for_alert = std::mem::take(&mut self.woke_for_alert);
        let rang = self.reminders.tick(dt, &self.robot.schedule, self.settings.clock24);
        if rang.is_empty() {
            return;
        }
        let woke = self.standby.state() != crate::standby::State::Awake || woke_for_alert;
        self.standby.wake_now();
        let top = self.stack.last().map(|(s, _)| (s.title() == "Reminders", s.full_screen()));
        let free = self.transition.is_none() && self.intro.is_none() && self.tour.is_none() && self.switcher.is_none() && !self.control.is_open();
        if woke && free && top == Some((false, false)) {
            self.open(Box::new(crate::reminders::Reminders::new()), Path::Slide);
            // One that opens something (its note, the checklist): that too,
            // over Reminders, so leaving it lands on the reminder to tick off.
            if let [one] = rang.as_slice() {
                if let Ok(screen) = crate::reminders::opens(&one.link) {
                    self.open(screen, Path::Slide);
                }
            }
        }
        let said = crate::reminders::island(&rang);
        // Kept as notifications (they break through Quiet).
        for ring in &rang {
            let n = crate::notify::Notification::said_as("Reminders", "checklist", &ring.said).urgent().action(format!("reminder:{}", ring.id));
            self.notes.add(n, false);
        }
        self.message = Some((said, REMINDER_SECONDS));
        self.lights.warning();
        crate::reminders::chime(&rang);
        self.dirty = true;
        self.moving = true;
    }

    /// Woken from deep standby: its steps undone in the reverse order. The
    /// screen comes on at once (that is standby's); catalyst-power runs on
    /// its own thread, and the steps taken before it wait for it.
    fn leave_deep(&mut self) {
        let reading = self.care.reading.clone();
        let Some(woke) = self.deep.end(reading.as_ref()) else { return };
        // The reminder began after the system went deep: out before it comes back.
        if woke.reminding {
            crate::lights::red_reminder(false);
        }
        for step in crate::deep::waking_first() {
            self.deep_step(step, false);
        }
        self.finish_waking();
        // How long, and what it drew (the charge is at the top right); once
        // unlocked, if it is locked (it is no notice for the lock screen).
        // From music standby nothing is said: the music says itself.
        if let Some(slept) = woke.slept {
            if self.lock.engaged() {
                self.after_unlock = Some(slept.island());
            } else {
                self.say(slept.island());
            }
        }
        // The clock and the battery at the top, read afresh for the first frame.
        self.status.age = f32::MAX;
        self.dirty = true;
        self.moving = true;
    }

    /// The system is back: the last of deep standby's steps undone.
    fn finish_waking(&mut self) {
        if !self.deep.ready_to_finish() {
            return;
        }
        for step in crate::deep::waking_after() {
            self.deep_step(step, false);
        }
        self.deep.woke();
    }

    /// Deep standby as if it had lasted `seconds` drawing what `reading`
    /// says, for the gallery and the tests.
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub fn deep_for(&mut self, seconds: f32, reading: crate::power::Reading) {
        if !self.deep.on() {
            self.enter_deep();
        }
        let step = 60.0;
        let mut left = seconds;
        while left > 0.0 {
            let dt = left.min(step);
            self.deep.sample(Some(&reading), dt);
            left -= dt;
        }
        self.care.reading = Some(reading);
    }

    /// Time to queue for the team's next match (FRC Nexus, when its key is
    /// set): the island says so five minutes ahead and again when it is
    /// time, and the keyboard's lights blink with each.
    fn watch_queue(&mut self, dt: f32) {
        self.queue_age += dt;
        if self.queue_age < 5.0 || self.robot.is_demo() {
            return;
        }
        self.queue_age = 0.0;
        let Some(view) = self.robot.schedule.view() else {
            self.queue_due = None;
            return;
        };
        let now = chrono::Utc::now().timestamp();
        self.queue_due = queue_alert_at(&view, now, &self.queue_warned);
        let Some(m) = view.next(now) else { return };
        let queuing = m.status.as_deref().is_some_and(|s| s.to_lowercase().contains("queu"));
        let stage = match (queuing, m.queue_at) {
            (true, _) => 1,
            (false, Some(at)) if now >= at && now <= at + 900 => 1,
            (false, Some(at)) if now >= at - QUEUE_AHEAD && now < at => 0,
            _ => return,
        };
        if self.queue_warned.iter().any(|(key, s)| *key == m.key && *s >= stage) {
            return;
        }
        self.queue_warned.push((m.key.clone(), stage));
        self.queue_due = queue_alert_at(&view, now, &self.queue_warned);
        let label = m.label.clone();
        // With the screen off, it wakes to say it (deep standby's minute
        // tick wakes it for this too).
        self.standby.wake_now();
        self.lights.warning();
        let n = if stage == 1 {
            crate::notify::Notification::new("Schedule", "flag", format!("time to queue for {label}")).urgent()
        } else {
            crate::notify::Notification::new("Schedule", "flag", format!("queue for {label} in 5 min"))
        };
        self.notify(n.tag(format!("queue:{}", m.key)).action("builtin:Schedule"));
    }

    /// At start: the start the tear-free panel is tried in asks, over
    /// everything, whether to keep it; a try not kept is said once.
    pub fn check_the_panel(&mut self) {
        if crate::panel::trial() == crate::panel::Trial::Trying {
            let calm = self.settings.calm;
            self.open(Box::new(crate::panel::KeepPanel::new(calm)), Path::Slide);
        } else if let Some(note) = crate::panel::not_kept_note() {
            self.say(note);
        }
    }

    /// The battery running out, as the gallery shows it.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn show_battery_empty(&mut self) {
        self.open(Box::new(crate::power::Empty::new(self.settings.calm)), Path::Slide);
    }

    /// A screen opened straight away, for the gallery.
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub fn show(&mut self, screen: Box<dyn Screen>) {
        self.open(screen, Path::Slide);
    }

    /// How deep the stack of screens is (for the tests of other modules).
    #[cfg(test)]
    pub fn stack_depth(&self) -> usize {
        self.stack.len()
    }

    /// What Home has in focus, where it is drawn (for the tests of other modules).
    #[cfg(test)]
    pub fn stack_spot(&self) -> Option<Rect> {
        self.stack.first().and_then(|(s, _)| s.spot())
    }

    /// The first picture is on the panel: the backlight, dark since power-on
    /// (so nothing the panel held before it was set up ever shows), rises
    /// to the level kept in the settings.
    pub fn light_up(&mut self) {
        self.standby.rise_from_dark(self.settings.brightness, self.settings.calm);
        self.moving = true;
    }

    /// The screen is off and dark: the panel itself can sleep.
    pub fn panel_off(&self) -> bool {
        self.standby.asleep()
    }

    /// Where the shell is, for the processor's boost (sched.rs): lit and in
    /// use; lit but dimming, going out or showing StandBy's faces (only a
    /// key raises it there); or dark.
    pub fn boost_place(&self) -> crate::sched::Place {
        if self.standby.asleep() || self.deep.on() {
            crate::sched::Place::Dark
        } else if self.standby.state() == crate::standby::State::Awake {
            crate::sched::Place::Lit
        } else {
            crate::sched::Place::Quiet
        }
    }

    /// Drawn over everything: a screenshot's flash, and (where there is no
    /// backlight to dim, as in the simulator) standby's dimming.
    fn draw_flash_and_standby(&mut self) {
        let px = &mut self.gfx.canvas.px;
        if let Some(flash) = &self.flash {
            let f = (flash.value().clamp(0.0, 1.0) * 0.8 * 256.0) as u32;
            if f > 0 {
                for p in px.iter_mut() {
                    *p = mix_px(*p, 0xffffff, f);
                }
            }
        }
        let shade = self.standby.shade();
        if shade > 0.0 {
            bezel::simd::mix_into(px, None, (shade.clamp(0.0, 1.0) * 256.0) as u32);
        }
    }
}

// -------------------------------------------------------------- spotlight ---

impl Shell {
    /// Whether the handheld is locked: the lock screen over everything (a
    /// passcode set), a lock screen anywhere on the stack, or a lock drawn
    /// over the screens (`spotlight::set_locked`). Nothing that reaches past
    /// it (Spotlight) opens while it is.
    pub fn locked(&self) -> bool {
        self.lock.covering() || crate::spotlight::lock_held() || self.stack.iter().any(|(s, _)| s.locks())
    }

    /// Bring Spotlight up over whatever is showing: not over a lock, nor a
    /// screen that takes the whole panel (first-boot setup). The control
    /// centre gives way to it at once.
    fn summon_spotlight(&mut self) {
        let whole_panel = self.stack.last().is_some_and(|(s, _)| s.full_screen());
        if self.locked() || whole_panel || self.tour.is_some() {
            return;
        }
        if self.control.visible() {
            self.control.set_open(false, &self.settings, &self.notes);
            self.control.shown.jump(0.0);
            self.control_under = None;
            self.control_glass = None;
        }
        // The clipboard's history in its place (shell/clip.rs).
        self.sheets.panel.gone_now();
        if !self.spotlight.visible() {
            self.spotlight_under = None;
        }
        self.spotlight.summon(self.settings.calm, &self.robot);
        self.moving = true;
        self.dirty = true;
    }

    /// Whether Spotlight is up (for the tests).
    #[cfg(test)]
    pub fn spotlight_open(&self) -> bool {
        self.spotlight.is_open()
    }

    fn spotlight_key(&mut self, key: Key) {
        use crate::spotlight::Outcome;
        self.moving = true;
        match key {
            // The recent apps, or the control centre, in its place.
            Key::Ctrl('\t') => {
                self.spotlight.gone_now();
                self.spotlight_under = None;
                return self.open_recents(None);
            }
            Key::Ctrl(' ') => {
                self.spotlight.gone_now();
                self.spotlight_under = None;
                return self.set_control(true);
            }
            _ => {}
        }
        // Copy what it found, paste into what is typed (shell/clip.rs).
        if self.spotlight_clipboard(key) {
            return;
        }
        match self.spotlight.key(key, &self.robot) {
            Outcome::Stay => {}
            Outcome::Close => self.spotlight.dismiss(),
            Outcome::Open(hit, from) => self.open_from_spotlight(&hit, from),
        }
    }

    /// A result picked: Spotlight goes, and what it opens grows out of the
    /// row it was on, over the frame as it showed. Esc from there comes back
    /// to the screen Spotlight came up over. What cannot open is said in the
    /// island, as the bar springs back into it.
    fn open_from_spotlight(&mut self, hit: &crate::spotlight::Hit, from: Rect) {
        let backdrop = self.gfx.canvas.px.clone();
        let mut say = None;
        let at_rest = self.transition.is_none();
        let opened = crate::spotlight::index().open(hit, &mut Cx { robot: &mut self.robot, settings: &mut self.settings, say: &mut say, at_rest });
        if let Some(text) = say {
            self.say(text);
        }
        let screen = match opened {
            Ok(screen) => screen,
            Err(why) => {
                self.spotlight.dismiss();
                return self.say(why);
            }
        };
        self.spotlight.gone_now();
        self.spotlight_under = None;
        // A screen still on its way gives way first (one leaving goes among
        // the recent apps): the new one grows over the still picture
        // Spotlight came down over, so no motion goes on beneath it.
        drop(self.interrupt());
        let app = hit.item.weight == crate::spotlight::weight::APP;
        self.open_over(screen, from, backdrop, app);
        self.apply_settings();
    }

    /// Open `screen` out of `from` over `backdrop`, on top of whatever shows.
    /// An app (`app`) opened over Home that was left a while ago comes back
    /// as it was, as from its tile.
    fn open_over(&mut self, screen: Box<dyn Screen>, from: Rect, backdrop: Vec<u32>, app: bool) {
        self.gfx.focus_reset();
        if let Some(words) = screen.greeting() {
            self.say(words);
        }
        let path = Path::Grow(from);
        if app && self.stack.len() == 1 {
            if let Some(mut recent) = self.recents.find(screen.title()).and_then(|at| self.recents.take(at)) {
                drop(screen);
                recent.stack[0].1 = path;
                return self.bring_back(recent, from, Some(backdrop));
            }
        }
        // Where it arrives over: closed while still arriving, it turns round
        // from there (see `close`) rather than leaving a motion with nothing
        // to bring in.
        let over = self.stack.len();
        self.push(screen, path);
        self.gfx.reveal_start();
        self.moving = true;
        self.dirty = true;
        if self.settings.calm {
            self.transition = None;
            return;
        }
        let mut transition = Transition::new(path, 0.0, 1.0, None, Some(backdrop));
        transition.over = over;
        self.transition = Some(transition);
    }

    /// Spotlight in time: its springs and results; gone at once if the
    /// handheld locks meanwhile; its held frame let go once it has gone; and
    /// its index set reading a moment after start, ahead of the first search.
    fn tick_spotlight(&mut self, motion_dt: f32) -> bool {
        if !self.spotlight_warmed && self.clock > 2.0 {
            self.spotlight_warmed = true;
            crate::spotlight::warm();
            crate::music::register_spotlight();
            crate::vault::register_spotlight();
        }
        let mut moving = false;
        if self.spotlight.visible() {
            if self.locked() {
                self.spotlight.gone_now();
            }
            moving |= self.spotlight.tick(motion_dt, &self.robot);
        }
        if !self.spotlight.visible() && self.spotlight_under.is_some() {
            self.spotlight_under = None;
            self.dirty = true;
        }
        moving
    }

    /// Spotlight over the frame held behind it: the frame dims a little as
    /// the bar comes, the bar's glass goes through one pass with the island
    /// and the page dots, and what sits on it is drawn on top.
    fn draw_spotlight(&mut self) {
        let began = std::time::Instant::now();
        let dim = (self.spotlight.dim().clamp(0.0, 1.0) * 256.0) as u32;
        if dim > 0 {
            bezel::simd::mix_into(&mut self.gfx.canvas.px, None, dim);
        }
        let island = self.island_rect();
        let bar = self.spotlight.glass(&self.gfx, island);
        self.draw_chrome(began, Some(bar), self.spotlight.shown());
        self.spotlight.draw(&mut self.gfx, island);
        if self.gfx.take_motion_started() {
            self.moving = true;
        }
    }
}

mod clip;
#[cfg(not(target_arch = "aarch64"))]
pub use clip::gallery as clip_gallery;
#[allow(unused_imports)]
pub use clip::Sheet;

#[cfg(test)]
mod fast_keys;

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
        // From the glance page, Enter opens what the widget nearest Home is
        // about (the next match, across the first row) out of it.
        shell.key(Key::Left);
        frames(&mut shell, 120, FRAME);
        shell.key(Key::Enter);
        let Some(Transition { path: Path::Grow(from), .. }) = &shell.transition else { panic!("the schedule opens out of the next match") };
        assert!(from.w > W * 0.5 && from.y < H * 0.5, "the wide widget at the top: {from:?}");
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

    /// A shell on Home in Flow, at rest.
    fn flow() -> Shell {
        let mut shell = shell();
        shell.settings.home = HomeStyle::Flow;
        shell.apply_settings();
        frames(&mut shell, 120, FRAME);
        shell
    }

    /// Where Home's focused card is: the middle one, or the enlarged one.
    fn spot(shell: &Shell) -> Rect {
        shell.stack[0].0.spot().expect("Home shows its cards")
    }

    #[test]
    fn flow_turns_the_card_over_opens_the_app_out_of_it_and_esc_steps_back() {
        let mut shell = flow();
        shell.key(Key::Right);
        shell.key(Key::Right);
        frames(&mut shell, 60, FRAME);
        assert_eq!(spot(&shell).w, 96.0, "the middle card");
        // Enter: the card comes forward and turns over; nothing opens yet.
        shell.key(Key::Enter);
        frames(&mut shell, 3, FRAME);
        assert!(shell.transition.is_none() && shell.stack.len() == 1);
        frames(&mut shell, 60, FRAME);
        let enlarged = spot(&shell);
        assert_eq!((enlarged.w, enlarged.h), (236.0, 116.0), "turned over and grown");
        // Enter again: the app opens out of the enlarged card.
        open(&mut shell, FRAME);
        let Some(Transition { path: Path::Grow(from), .. }) = &shell.transition else { panic!("the app grows out of the card") };
        assert_eq!(*from, enlarged);
        assert_eq!(shell.stack.last().unwrap().0.title(), "Logs");
        frames(&mut shell, 60, FRAME);
        // Esc in the app: it goes back into the card, which then turns back
        // into the flow, still on Logs.
        shell.key(Key::Back);
        frames(&mut shell, 3, FRAME);
        assert!(matches!(&shell.transition, Some(Transition { path: Path::Grow(into), .. }) if *into == enlarged), "back into the enlarged card");
        frames(&mut shell, 90, FRAME);
        assert_eq!(shell.stack.len(), 1);
        assert_eq!(spot(&shell).w, 96.0, "turned back into the flow");
        // Esc on an enlarged card turns it back, and the flow stays where it is.
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        shell.key(Key::Back);
        frames(&mut shell, 60, FRAME);
        assert_eq!(spot(&shell).w, 96.0);
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        open(&mut shell, FRAME);
        assert_eq!(shell.stack.last().unwrap().0.title(), "Logs", "the flow stayed on Logs");
        frames(&mut shell, 60, FRAME);
        shell.key(Key::Back);
        frames(&mut shell, 120, FRAME);
        // Esc in the flow: back to the first app, gliding there.
        shell.key(Key::Back);
        frames(&mut shell, 120, FRAME);
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        open(&mut shell, FRAME);
        assert_eq!(shell.stack.last().unwrap().0.title(), "Robot");
        // The calm version: no glide, no turning, and the same steps.
        let mut shell = flow();
        shell.settings.calm = true;
        shell.apply_settings();
        shell.key(Key::Right);
        shell.key(Key::Enter);
        frames(&mut shell, 1, FRAME);
        assert_eq!(spot(&shell).w, 236.0, "turned over at once");
        shell.key(Key::Enter);
        frames(&mut shell, 1, FRAME);
        assert_eq!(title(&shell), "Health");
    }

    #[test]
    fn over_the_flow_the_control_centre_spotlight_recents_and_a_robot_linking_all_work() {
        // Up: the control centre comes down; Esc puts it away.
        let mut shell = flow();
        shell.key(Key::Up);
        frames(&mut shell, 60, FRAME);
        assert!(shell.control_open());
        shell.key(Key::Back);
        frames(&mut shell, 60, FRAME);
        assert!(!shell.control_open());
        // Space: Spotlight, over it; Esc back to the flow.
        shell.key(Key::Char(' '));
        frames(&mut shell, 60, FRAME);
        assert!(shell.spotlight_open());
        shell.key(Key::Back);
        frames(&mut shell, 60, FRAME);
        assert!(!shell.spotlight_open());
        // An app left is among the recent apps, over the flow.
        shell.key(Key::Enter);
        frames(&mut shell, 30, FRAME);
        open(&mut shell, FRAME);
        frames(&mut shell, 60, FRAME);
        shell.key(Key::Back);
        frames(&mut shell, 90, FRAME);
        shell.key(Key::Ctrl('\t'));
        frames(&mut shell, 60, FRAME);
        assert!(shell.recents_open());
        shell.key(Key::Back);
        frames(&mut shell, 60, FRAME);
        // A robot linking while the flow is turned away from Robot: it turns
        // there, then Robot opens out of the middle card.
        let mut shell = waiting();
        shell.settings.home = HomeStyle::Flow;
        shell.apply_settings();
        frames(&mut shell, 60, FRAME);
        for _ in 0..4 {
            shell.key(Key::Right);
        }
        frames(&mut shell, 120, FRAME);
        shell.robot = Robot::demo();
        frames(&mut shell, 120, FRAME);
        let Some((screen, Path::Grow(from))) = shell.stack.last() else { panic!("out of the middle card") };
        assert_eq!(screen.title(), "Robot");
        assert_eq!((from.cx(), from.w), (W * 0.5, 96.0));
        // And the page track: the place along it, as the flow turns.
        let mut shell = flow();
        shell.draw();
        assert!(shell.places.pager.is_some_and(|p| (p.w - SCRUB_W).abs() < 0.5), "a track, not 39 dots");
        // At rest nothing moves: no frames are asked for (only what the
        // robot or the clock says may still be drawn), and standby comes as ever.
        shell.key(Key::Right);
        frames(&mut shell, 90, FRAME);
        shell.tick(FRAME);
        assert!(!shell.moving() && !shell.animating(), "the flow at rest asks for no frames");
        let mut shell = flow();
        shell.settings.screen_off = 30;
        frames(&mut shell, 40 * 4, 0.25);
        assert_eq!(shell.standby_state(), crate::standby::State::Off, "the screen dims and goes off over the flow");
        shell.key(Key::Enter);
        frames(&mut shell, 30, FRAME);
        assert_eq!(spot(&shell).w, 96.0, "the key that wakes it does not also turn the card over");
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
        // Set up here; past Wi-Fi, the hardware check and the keyboard lesson; the team.
        shell.key(Key::Enter);
        shell.key(Key::Tab);
        shell.key(Key::Tab);
        shell.key(Key::Tab);
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
        // Set up here; no Wi-Fi now; the hardware check and the keyboard
        // lesson skipped; team 5805; the name and time zone as they are; keys
        // skipped; choices: the first off; power as it is.
        shell.key(Key::Enter);
        shell.key(Key::Down);
        shell.key(Key::Enter);
        shell.key(Key::Tab);
        shell.key(Key::Tab);
        for c in "5805".chars() {
            shell.key(Key::Char(c));
        }
        shell.key(Key::Enter);
        shell.key(Key::Tab);
        shell.key(Key::Tab);
        // Choices start on Next (Enter goes on): up to the first, switched off.
        assert!(shell.settings.open_robot);
        for _ in 0..4 {
            shell.key(Key::Up);
        }
        shell.key(Key::Enter);
        assert!(!shell.settings.open_robot, "the first choice switched off");
        shell.key(Key::Tab);
        shell.key(Key::Tab);
        // The look, also on Next: up to the accent, the next one, light mode, then Finish.
        for _ in 0..4 {
            shell.key(Key::Up);
        }
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
    fn the_glance_page_runs_left_in_pages_of_two_rows() {
        use crate::widgets::{Kind, Placed, Size};
        let mut shell = shell();
        // The robot and a pinned value above the next match; the last match
        // large, a page of its own, to the left of that.
        let w = |kind, size| Placed::new(kind, size, 0);
        shell.settings.widgets.glance = vec![w(Kind::Robot, Size::Small), w(Kind::Pinned, Size::Small), w(Kind::NextMatch, Size::Wide), w(Kind::LastMatch, Size::Large)];
        shell.apply_settings();
        shell.key(Key::Left);
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Glance");
        // On the pinned value first, the nearer one; left to the robot.
        shell.key(Key::Left);
        // Left again: the next page, the last match.
        shell.key(Key::Left);
        frames(&mut shell, 60, FRAME);
        shell.key(Key::Enter);
        frames(&mut shell, 120, FRAME);
        assert_eq!(title(&shell), "Match", "the last match opens its summary");
        shell.key(Key::Back);
        frames(&mut shell, 120, FRAME);
        // Right: back onto the first page, at its left; Down, the next match below.
        shell.key(Key::Right);
        shell.key(Key::Down);
        shell.key(Key::Enter);
        frames(&mut shell, 120, FRAME);
        assert_eq!(title(&shell), "Schedule");
        shell.key(Key::Back);
        frames(&mut shell, 120, FRAME);
        // Up to the robot; Up again from the top row is the control centre.
        shell.key(Key::Up);
        assert!(!shell.control_open());
        shell.key(Key::Up);
        assert!(shell.control_open());
        shell.key(Key::Up);
        frames(&mut shell, 120, FRAME);
        // Right twice: across the first page's top row, then Home.
        for _ in 0..2 {
            shell.key(Key::Right);
        }
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Catalyst");
        // A pinned value, chosen in Tables, has a widget of its own.
        shell.settings.pinned = Some("Loop/Robot/LastMs".into());
        shell.settings.widgets.glance = vec![w(Kind::Pinned, Size::Small)];
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
            Key::Ctrl('q'), Key::Ctrl(' '), Key::Ctrl('s'), Key::Ctrl('k'), Key::Leave, Key::Ctrl('\n'), Key::Char('/'),
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
    fn question_mark_held_five_seconds_brings_up_the_shortcuts() {
        let mut shell = shell();
        frames(&mut shell, 30, FRAME);
        // Held, a key keeps coming: the first repeat 0.4 s in, then every
        // 75 ms (here a little slower, every four frames).
        let hold = |shell: &mut Shell, seconds: f32| {
            shell.key(Key::Char('?'));
            frames(shell, 24, FRAME);
            let mut held = 0.4;
            while held < seconds {
                shell.key(Key::Char('?'));
                frames(shell, 4, FRAME);
                held += 4.0 * FRAME;
            }
        };
        // Typed in passing, even a few at once: nothing at all.
        for _ in 0..3 {
            shell.key(Key::Char('?'));
            frames(&mut shell, 40, FRAME);
        }
        assert_eq!(title(&shell), "Catalyst");
        assert!(shell.message.is_none(), "not even a word");
        // Held three seconds: not yet, but the island says what holding on will do.
        hold(&mut shell, 3.0);
        assert_eq!(title(&shell), "Catalyst");
        assert!(shell.message.as_ref().is_some_and(|(m, _)| m == "shortcuts in 2 s" || m == "shortcuts in 3 s"), "{:?}", shell.message);
        frames(&mut shell, 60, FRAME);
        assert!(shell.message.is_none(), "let go, the countdown goes");
        // Held five: the list.
        hold(&mut shell, 5.3);
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Shortcuts");
        // Held again over it: it stays, and no second list comes.
        hold(&mut shell, 6.0);
        frames(&mut shell, 60, FRAME);
        assert_eq!((title(&shell), shell.stack.len()), ("Shortcuts".to_owned(), 2));
        shell.key(Key::Back);
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Catalyst");
        // Where text is typed, ? is typed, held or not.
        shell.key(Key::Char(' '));
        frames(&mut shell, 60, FRAME);
        let searching = title(&shell);
        hold(&mut shell, 5.5);
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

    /// The screen in front's own address: the same app, or a new copy.
    fn which(shell: &Shell) -> usize {
        &*shell.stack.last().unwrap().0 as *const dyn Screen as *const u8 as usize
    }

    /// Home's focus moved from the built-in app called `from` to `name`,
    /// and that app opened out of its tile.
    fn open_app(shell: &mut Shell, from: &str, name: &str) {
        let place = |n: &str| crate::screens::builtin_apps().position(|(_, _, a)| a == n).unwrap() as i32;
        let steps = place(name) - place(from);
        for _ in 0..steps.abs() {
            shell.key(if steps > 0 { Key::Right } else { Key::Left });
        }
        open(shell, FRAME);
        frames(shell, 60, FRAME);
        assert_eq!(title(shell), name);
    }

    fn back_home(shell: &mut Shell) {
        while shell.stack.len() > 1 {
            shell.key(Key::Back);
            frames(shell, 60, FRAME);
        }
    }

    #[test]
    fn an_app_left_is_kept_and_comes_back_as_it_was() {
        let mut shell = shell();
        open_app(&mut shell, "Robot", "Settings");
        let settings = which(&shell);
        shell.key(Key::Back);
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Catalyst");
        assert_eq!(shell.recents.find("Settings"), Some(0), "kept");
        // Opened again from its tile (the focus is still on it): the same
        // Settings, as it was left.
        open(&mut shell, FRAME);
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Settings");
        assert_eq!(which(&shell), settings, "not a new copy");
        assert_eq!(shell.recents.find("Settings"), None, "in front, so not among the recent apps");
    }

    #[test]
    fn ctrl_tab_goes_back_and_forth_between_two_apps() {
        let mut shell = shell();
        open_app(&mut shell, "Robot", "Robot");
        let robot = which(&shell);
        back_home(&mut shell);
        open_app(&mut shell, "Robot", "Health");
        let health = which(&shell);
        // Ctrl-Tab, Enter: the app before.
        shell.key(Key::Ctrl('\t'));
        frames(&mut shell, 30, FRAME);
        assert!(shell.recents_open());
        shell.key(Key::Enter);
        assert!(!shell.recents_open());
        frames(&mut shell, 60, FRAME);
        assert_eq!((title(&shell), which(&shell)), ("Robot".to_owned(), robot));
        assert_eq!(shell.stack.len(), 2, "in place of Health, not over it");
        // And back again.
        shell.key(Key::Ctrl('\t'));
        frames(&mut shell, 30, FRAME);
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        assert_eq!((title(&shell), which(&shell)), ("Health".to_owned(), health));
        // Enter on the app in front: the cards go, and there it is.
        shell.key(Key::Ctrl('\t'));
        frames(&mut shell, 30, FRAME);
        shell.key(Key::Left);
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        assert_eq!((title(&shell), which(&shell), shell.recents_open()), ("Health".to_owned(), health, false));
    }

    #[test]
    fn the_recent_apps_come_up_over_a_terminal_and_let_apps_go() {
        let mut shell = shell();
        open_app(&mut shell, "Robot", "Logs");
        back_home(&mut shell);
        open_app(&mut shell, "Logs", "Terminal");
        // Ctrl-Tab is the shell's, even over a terminal that takes every key.
        shell.key(Key::Ctrl('\t'));
        frames(&mut shell, 30, FRAME);
        assert!(shell.recents_open());
        // Let Logs go (the card after the terminal's own).
        shell.key(Key::Up);
        frames(&mut shell, 30, FRAME);
        assert_eq!(shell.recents.find("Logs"), None, "let go");
        // Let the terminal go too: Home behind the cards, nothing kept.
        shell.key(Key::Left);
        shell.key(Key::Up);
        frames(&mut shell, 30, FRAME);
        assert_eq!(shell.stack.len(), 1);
        shell.key(Key::Back);
        frames(&mut shell, 60, FRAME);
        assert_eq!((title(&shell), shell.recents_open()), ("Catalyst".to_owned(), false));
        // From Home with nothing kept: the cards say so, and Esc puts them away.
        shell.key(Key::Ctrl('\t'));
        frames(&mut shell, 30, FRAME);
        assert!(shell.recents_open());
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        assert!(!shell.recents_open());
    }

    #[test]
    fn a_screenshot_is_taken_from_the_control_centre() {
        let mut shell = shell();
        frames(&mut shell, 30, FRAME);
        let folder = crate::screenshot::folder();
        let count = || std::fs::read_dir(&folder).map_or(0, |d| d.count());
        let before = count();
        shell.key(Key::Ctrl(' '));
        frames(&mut shell, 40, FRAME);
        shell.key(Key::Char('s'));
        assert!(shell.flash.is_some(), "a flash says it was taken");
        assert!(!shell.control_open(), "and the sheet goes");
        // It is written on a thread of its own: wait for it, in real time.
        for _ in 0..300 {
            frames(&mut shell, 1, FRAME);
            if shell.saving.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("screenshot saved"));
        assert_eq!(count(), before + 2, "at the panel's size and twice it");
    }

    #[test]
    fn no_screenshot_is_taken_of_the_vault_open() {
        let mut shell = shell();
        crate::passcode::test_store();
        let (phc, _, _) = crate::passcode::make("2468").unwrap();
        crate::passcode::keep(&phc).unwrap();
        match crate::vault::begin_create(zeroize::Zeroizing::new("2468".into())).recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(crate::vault::Outcome::Opened(opened)) => opened.install(),
            _ => panic!("the vault was not made"),
        }
        shell.open(Box::new(crate::vault::Vault::new()), Path::Slide);
        frames(&mut shell, 40, FRAME);
        assert!(crate::vault::is_open());
        shell.key(Key::Ctrl(' '));
        frames(&mut shell, 40, FRAME);
        shell.key(Key::Char('s'));
        assert!(shell.flash.is_none() && shell.saving.is_none(), "nothing taken");
        assert!(!shell.control_open(), "the sheet goes all the same");
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("no screenshots of the vault"));
        crate::vault::lock();
    }

    #[test]
    fn standby_dims_turns_the_screen_off_and_the_waking_key_is_spent() {
        use crate::standby::State;
        let mut shell = shell();
        shell.settings.screen_off = 30;
        // Half a minute: it dims eight seconds before (the shell is two seconds old).
        frames(&mut shell, 18 * 4, 0.25);
        assert_eq!(shell.standby_state(), State::Awake);
        frames(&mut shell, 4 * 4, 0.25);
        assert_eq!(shell.standby_state(), State::Dim);
        frames(&mut shell, 10 * 4, 0.25);
        assert_eq!(shell.standby_state(), State::Off);
        assert!(!shell.tick(0.25), "off and dark: nothing is drawn");
        // Enter wakes it, and does not also open the tile in focus.
        shell.key(Key::Enter);
        frames(&mut shell, 30, FRAME);
        assert_eq!((shell.standby_state(), shell.stack.len()), (State::Awake, 1));
        // What runs in a terminal printing keeps it awake past the wait.
        open_app(&mut shell, "Robot", "Terminal");
        for key in [Key::Char('l'), Key::Char('s'), Key::Enter] {
            shell.key(key);
        }
        frames(&mut shell, 40 * 4, 0.25);
        assert!(shell.stack.last().unwrap().0.keeps_awake());
        assert_eq!(shell.standby_state(), State::Awake);
    }

    #[test]
    fn the_on_device_model_is_let_go_once_the_screen_is_off() {
        use crate::standby::State;
        let mut shell = shell();
        shell.settings.screen_off = 30;
        // A model loaded (this process stands in for it).
        crate::local::pretend_loaded(Some(std::process::id()));
        frames(&mut shell, 32 * 4, 0.25);
        assert_eq!(shell.standby_state(), State::Off);
        assert!(crate::local::let_go(), "the screen off: the model is to go");
        crate::local::pretend_loaded(None);
    }

    /// Seconds of ticks, one a second (as the main loop's idle looks add
    /// up), until deep standby or `most` seconds.
    fn until_deep(shell: &mut Shell, most: u32) -> Option<u32> {
        for s in 1..=most {
            shell.tick(1.0);
            if shell.deep_asleep() {
                return Some(s);
            }
        }
        None
    }

    // ----------------------------------------------- StandBy and the lock ---

    /// A shell with `code` as its passcode (kept for this test alone).
    pub(crate) fn with_passcode(code: &str) -> Shell {
        crate::passcode::test_store();
        let (phc, _, _) = crate::passcode::make(code).unwrap();
        crate::passcode::keep(&phc).unwrap();
        shell()
    }

    /// Type a passcode and Enter, and wait for it to be checked.
    pub(crate) fn type_code(shell: &mut Shell, code: &str) {
        for c in code.chars() {
            shell.key(Key::Char(c));
        }
        shell.key(Key::Enter);
        // Up to 10 s: the check runs on its own thread, which a computer
        // busy with other builds can be slow to get to.
        for _ in 0..2000 {
            frames(shell, 1, FRAME);
            if !shell.lock.checking() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        frames(shell, 40, FRAME);
    }

    /// Seconds of ticks, one a second, until the screen is off and dark.
    fn until_off(shell: &mut Shell, most: u32) -> Option<u32> {
        (1..=most).find(|_| {
            shell.tick(1.0);
            shell.draw();
            shell.panel_off()
        })
    }

    #[test]
    fn on_the_charger_standby_comes_where_the_screen_would_go_off_and_deep_standby_never_begins() {
        use crate::standby::State;
        let mut shell = shell();
        shell.settings.screen_off = 30;
        shell.settings.battery.deep_after = 10;
        shell.pretend_charger(Some(true));
        // On the charger: StandBy where the screen would have gone off.
        for _ in 0..60 {
            shell.tick(1.0);
        }
        assert_eq!(shell.standby_state(), State::Faces);
        assert!(shell.faces_showing() && !shell.panel_off());
        assert!(!shell.locked(), "no passcode: no lock");
        // At rest it keeps no frame beat: a face is drawn when it changes.
        frames(&mut shell, 4, 0.25);
        assert!(!shell.animating(), "nothing moving at rest");
        // ↓ turns to the next face (and is kept); it stays StandBy.
        shell.key(Key::Down);
        assert_eq!(shell.settings.lock.face, crate::lock::Face::Music);
        assert!(shell.animating(), "the page turn runs on the frame beat");
        assert_eq!(shell.frame_floor(), Some(crate::power::SAVER_FRAME_MS), "at most 30 frames a second");
        frames(&mut shell, 60, FRAME);
        assert_eq!(shell.standby_state(), State::Faces);
        // Any other key leaves it, and is spent doing so: Home, as it was.
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        assert_eq!(shell.standby_state(), State::Awake);
        assert!(!shell.faces_showing());
        assert_eq!(shell.stack.len(), 1, "the Enter that left StandBy opened nothing");
        // All night on the charger: StandBy for a while, then dark (a
        // handheld left on a computer's USB does not stay lit), never deep.
        assert_eq!(until_deep(&mut shell, 8 * 3600), None, "deep standby never begins on the charger");
        assert_eq!(shell.standby_state(), State::Off);
        assert!(!shell.faces_showing());
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        assert_eq!(shell.standby_state(), State::Awake);
        // StandBy switched off: the screen goes off on the charger as ever.
        shell.settings.lock.standby = false;
        assert!(until_off(&mut shell, 60).is_some());
        assert!(!shell.faces_showing());
    }

    #[test]
    fn off_the_charger_standby_goes_dark_and_deep_standby_keeps_its_wait() {
        use crate::standby::State;
        let mut shell = shell();
        shell.settings.screen_off = 30;
        shell.settings.battery.deep_after = 10;
        shell.pretend_charger(Some(true));
        for _ in 0..120 {
            shell.tick(1.0);
        }
        assert_eq!(shell.standby_state(), State::Faces);
        // Off the charger: dark, as the screen would have gone without it.
        shell.pretend_charger(Some(false));
        frames(&mut shell, 8, 0.25);
        assert_eq!(shell.standby_state(), State::Off);
        assert!(!shell.faces_showing());
        // Deep standby follows its own wait from there, not from the last
        // key hours ago: ten minutes less the half minute the screen took.
        let at = until_deep(&mut shell, 900).expect("deep standby");
        assert!((560..=585).contains(&at), "deep {at} s after leaving the charger");
    }

    #[test]
    fn set_down_on_the_charger_with_the_screen_off_standby_comes_at_once() {
        use crate::standby::State;
        let mut shell = shell();
        shell.settings.screen_off = 30;
        shell.pretend_charger(Some(false));
        until_off(&mut shell, 60).expect("the screen goes off on battery");
        frames(&mut shell, 4, 0.25);
        shell.pretend_charger(Some(true));
        frames(&mut shell, 2, 0.25);
        assert_eq!(shell.standby_state(), State::Faces, "on the charger: StandBy, lit");
        assert!(shell.faces_showing() && !shell.panel_off());
        // In use, the charger changes nothing until it is left alone.
        shell.key(Key::Enter);
        frames(&mut shell, 20, FRAME);
        shell.pretend_charger(Some(false));
        frames(&mut shell, 2, 0.25);
        shell.pretend_charger(Some(true));
        frames(&mut shell, 8, 0.25);
        assert_eq!(shell.standby_state(), State::Awake);
        // From deep standby, at its next look (a minute at most).
        shell.pretend_charger(Some(false));
        shell.settings.battery.deep_after = 10;
        until_deep(&mut shell, 700).expect("deep standby");
        shell.pretend_charger(Some(true));
        shell.tick(60.0);
        assert!(!shell.deep_asleep(), "out of deep standby");
        frames(&mut shell, 4, 0.25);
        assert_eq!(shell.standby_state(), State::Faces);
    }

    #[test]
    fn with_no_passcode_a_key_wakes_it_straight_to_where_it_was() {
        let mut shell = shell();
        shell.settings.screen_off = 30;
        open_app(&mut shell, "Robot", "Settings");
        let depth = shell.stack.len();
        until_off(&mut shell, 60).expect("the screen goes off");
        assert!(!shell.locked() && !shell.lock.visible());
        shell.key(Key::Char(' '));
        frames(&mut shell, 30, FRAME);
        assert_eq!((shell.stack.len(), shell.top_title()), (depth, "Settings"), "straight back, no lock screen");
    }

    #[test]
    fn with_a_passcode_it_starts_locked_and_locks_when_the_screen_goes_off() {
        let mut shell = with_passcode("2468");
        assert!(shell.locked(), "a restart is no way in");
        // The wrong one: still locked, the field shaken and emptied.
        type_code(&mut shell, "1357");
        assert!(shell.locked());
        assert!(shell.lock.field.is_empty());
        // The right one: open, at Home as it was.
        type_code(&mut shell, "2468");
        assert!(!shell.locked());
        frames(&mut shell, 60, FRAME);
        assert!(!shell.lock.visible(), "the lock screen has gone");
        open_app(&mut shell, "Robot", "Settings");
        // The screen dims first: a key then is only a key.
        shell.settings.screen_off = 30;
        for _ in 0..24 {
            shell.tick(1.0);
        }
        assert_eq!(shell.standby_state(), crate::standby::State::Dim);
        assert!(!shell.locked(), "dimmed is not off");
        shell.key(Key::Char(' '));
        frames(&mut shell, 20, FRAME);
        assert!(!shell.locked());
        // Off: locked. The waking key is spent; the next ones are the passcode's.
        until_off(&mut shell, 60).expect("the screen goes off");
        assert!(shell.locked());
        shell.key(Key::Enter);
        frames(&mut shell, 20, FRAME);
        assert!(shell.lock.covers(), "the lock screen, over everything");
        assert_eq!(shell.top_title(), "Settings", "where it was, beneath");
        type_code(&mut shell, "2468");
        assert!(!shell.locked());
        assert_eq!(shell.top_title(), "Settings");
    }

    #[test]
    fn the_lock_cannot_be_got_round_from_any_key() {
        let mut shell = with_passcode("hunter2");
        type_code(&mut shell, "hunter2");
        open_app(&mut shell, "Robot", "Settings");
        shell.settings.developer = true;
        shell.settings.show_keys = true;
        shell.settings.screen_off = 30;
        until_off(&mut shell, 60).expect("the screen goes off");
        shell.key(Key::Char(' '));
        let depth = shell.stack.len();
        let mut keys = vec![
            Key::Ctrl('\t'), Key::Ctrl(' '), Key::Ctrl('q'), Key::Ctrl('c'), Key::Ctrl('k'), Key::Ctrl('f'),
            Key::Leave, Key::Back, Key::Tab, Key::Home, Key::End, Key::PageUp, Key::PageDown, Key::Delete,
            Key::Up, Key::Down, Key::Left, Key::Right, Key::BrightnessUp, Key::BrightnessDown,
            Key::Char('/'), Key::Char('?'), Key::Char('`'), Key::Enter, Key::Ctrl('\n'), Key::Lock, Key::Ctrl(crate::platform::NOTIFICATIONS),
            // The clipboard's: copy, paste, its history, quick capture.
            Key::Ctrl('C'), Key::Ctrl('v'), Key::Ctrl('V'), Key::Ctrl('x'), Key::Ctrl('H'), Key::Ctrl('N'), Key::Ctrl('P'),
        ];
        keys.extend((1..=12).map(Key::F));
        crate::clipboard::copy("on the clipboard", crate::clipboard::Sensitivity::Normal);
        for key in keys {
            shell.key(key);
            frames(&mut shell, 3, FRAME);
            assert!(shell.locked(), "{key:?} got round the lock");
            assert!(!shell.recents_open() && !shell.control_open(), "{key:?} opened something over the lock");
            assert!(!shell.spotlight_open() && !shell.spotlight.visible(), "{key:?} brought Spotlight up over the lock");
            assert!(!shell.sheets.visible(), "{key:?} brought the clipboard or quick capture up over the lock");
            // Only `/`, `?` and `` ` `` are typed there: never a paste.
            assert!(shell.lock.field.len() <= 3, "{key:?} pasted into the passcode");
            assert!(shell.tour.is_none() && shell.echo.is_empty(), "{key:?}");
            assert_eq!(shell.stack.len(), depth, "{key:?} reached the screen beneath");
            assert!(shell.take_launch().is_none());
        }
        // `?` held five seconds (the shortcut list) is only typing.
        for _ in 0..60 {
            shell.key(Key::Char('?'));
            frames(&mut shell, 6, FRAME);
        }
        assert!(shell.locked() && shell.stack.len() == depth);
        // A robot linking, a match to queue for, the battery: still locked.
        shell.notify(crate::notify::Notification::new("Schedule", "flag", "time to queue for Q31").urgent());
        frames(&mut shell, 10, 0.25);
        assert!(shell.locked());
        assert_eq!(shell.lock_info().notes.len(), 1, "on the lock screen");
        // Typing it, with every wrong key first cleared by Esc: open.
        shell.key(Key::Back);
        type_code(&mut shell, "hunter2");
        assert!(!shell.locked());
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("time to queue for Q31"), "said once unlocked");
        assert_eq!(shell.notes.unread(), 1, "not seen until the notification centre shows it");
    }

    #[test]
    fn push_to_talk_never_listens_or_asks_under_the_lock() {
        let mut shell = with_passcode("2468");
        assert!(shell.locked(), "a passcode: locked from the start");
        // The talk key held and let go under the lock screen: no microphone,
        // no assistant opened beneath it, still locked.
        crate::voice::set_talk_key(true);
        frames(&mut shell, 60, FRAME);
        assert!(!shell.voice.listening(), "no listening under the lock");
        crate::voice::set_talk_key(false);
        frames(&mut shell, 120, FRAME);
        assert!(shell.stack.iter().all(|(s, _)| s.title() != "Assistant"), "nothing asked past the passcode");
        assert!(shell.locked());
        // Unlocked, it listens as ever.
        type_code(&mut shell, "2468");
        assert!(!shell.locked());
        crate::voice::set_talk_key(true);
        frames(&mut shell, 60, FRAME);
        assert!(shell.voice.listening());
        crate::voice::set_talk_key(false);
    }

    #[test]
    fn wrong_tries_bring_a_wait_that_even_the_right_one_cannot_skip() {
        let mut shell = with_passcode("2468");
        for wrong in ["0000", "1111", "2222", "3333"] {
            type_code(&mut shell, wrong);
            assert_eq!(shell.lock.wait(), 0, "{wrong}: no wait yet");
        }
        type_code(&mut shell, "4444");
        // Half a minute, counting down on the real clock (a busy computer
        // can take a second or two over the check).
        assert!((27..=30).contains(&shell.lock.wait()), "the fifth wrong one: half a minute, not {}", shell.lock.wait());
        // During the wait no try is taken, the right one included.
        type_code(&mut shell, "2468");
        assert!(shell.locked() && !shell.lock.checking());
        // A restart keeps the wait.
        let again = Shell::new(Robot::demo());
        assert!(again.locked() && again.lock.wait() > 25);
    }

    #[test]
    fn standby_with_a_passcode_leaves_to_the_lock_screen() {
        let mut shell = with_passcode("2468");
        type_code(&mut shell, "2468");
        shell.settings.screen_off = 30;
        shell.pretend_charger(Some(true));
        for _ in 0..120 {
            shell.tick(1.0);
            shell.draw();
        }
        assert!(shell.faces_showing());
        assert!(shell.locked(), "StandBy stands in for the screen going off");
        // The arrows turn faces; nothing else is reached.
        shell.key(Key::Down);
        assert!(shell.faces_showing() && shell.locked());
        // Leaving StandBy: the lock screen, not the screen it was on.
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        assert!(!shell.faces_showing() && shell.lock.covers());
        type_code(&mut shell, "2468");
        assert!(!shell.locked());
    }

    /// Where the shell is, to tell it came back to the same place.
    fn place(shell: &Shell) -> (usize, String, bool, bool, bool) {
        (shell.stack.len(), shell.top_title().to_owned(), shell.spotlight_open(), shell.control_open(), shell.recents_open())
    }

    #[test]
    fn tab_held_locks_from_every_context_and_any_key_lifts_it_without_a_passcode() {
        use crate::standby::State;
        // (what to do to get there, what is open over the screen)
        type Setup = fn(&mut Shell);
        let contexts: [(&str, Setup); 7] = [
            ("home", |_| {}),
            ("the terminal", |s| open_app(s, "Robot", "Terminal")),
            ("settings", |s| open_app(s, "Robot", "Settings")),
            ("spotlight", |s| {
                s.key(Key::Ctrl('\n'));
                frames(s, 30, FRAME);
            }),
            ("the control centre", |s| {
                s.key(Key::Ctrl(' '));
                frames(s, 30, FRAME);
            }),
            ("the recent apps", |s| {
                open_app(s, "Robot", "Settings");
                back_home(s);
                s.key(Key::Ctrl('\t'));
                frames(s, 30, FRAME);
            }),
            ("a dimmed screen", |s| {
                s.settings.screen_off = 30;
                for _ in 0..24 {
                    s.tick(1.0);
                }
            }),
        ];
        for (name, setup) in contexts {
            let mut shell = shell();
            setup(&mut shell);
            let overlay = shell.spotlight_open() || shell.control_open() || shell.recents_open();
            let before = place(&shell);
            shell.key(Key::Lock);
            frames(&mut shell, 40, FRAME);
            assert!(shell.lock.covering() && !shell.lock.engaged(), "{name}: shown, not locked (no passcode)");
            assert!(shell.locked(), "{name}: nothing opens over it");
            assert_eq!(shell.standby_state(), State::Awake, "{name}");
            assert!(!shell.spotlight_open() && !shell.control_open() && !shell.recents_open(), "{name}: nothing left open over the screen");
            // Spotlight, the recent apps and the control centre do not open over it.
            for key in [Key::Ctrl('\n'), Key::Ctrl('\t'), Key::Ctrl(' ')] {
                let mut again = shell_like(&shell);
                again.key(key);
                frames(&mut again, 10, FRAME);
                assert!(!again.spotlight_open() && !again.control_open() && !again.recents_open(), "{name}: {key:?} over the lock");
            }
            // Held again while shown: nothing.
            shell.key(Key::Lock);
            assert!(shell.lock.covering());
            // Any key lifts it, and is spent: back where it was.
            shell.key(Key::Char('a'));
            frames(&mut shell, 60, FRAME);
            assert!(!shell.lock.covering() && !shell.lock.visible(), "{name}: lifted");
            let after = place(&shell);
            assert_eq!((after.0, after.1), (before.0, before.1), "{name}: back where it was");
            if overlay {
                assert!(!after.2 && !after.3 && !after.4, "{name}: what was over the screen stays put away");
            }
        }
    }

    /// A shell with the same lock shown, for a key tried on it alone.
    fn shell_like(shown: &Shell) -> Shell {
        let mut s = shell();
        if shown.lock.covering() {
            s.key(Key::Lock);
            frames(&mut s, 20, FRAME);
        }
        s
    }

    #[test]
    fn tab_held_locks_from_off_standby_and_deep_standby_and_needs_the_passcode() {
        use crate::standby::State;
        let mut shell = with_passcode("2468");
        type_code(&mut shell, "2468");
        assert!(!shell.locked());
        // Off: woken onto the lock screen, locked.
        shell.settings.screen_off = 30;
        until_off(&mut shell, 60).expect("the screen goes off");
        shell.key(Key::Lock);
        frames(&mut shell, 30, FRAME);
        assert_eq!(shell.standby_state(), State::Awake, "held in the dark, it wakes");
        assert!(shell.lock.engaged() && shell.lock.visible());
        // A tap of Tab on the lock screen is no way out either.
        shell.key(Key::Tab);
        assert!(shell.lock.engaged());
        type_code(&mut shell, "2468");
        assert!(!shell.locked());
        // From the terminal, with text being typed: locked, and Tab (a tap)
        // is still the terminal's afterwards.
        open_app(&mut shell, "Robot", "Terminal");
        shell.key(Key::Char('l'));
        shell.key(Key::Tab);
        assert!(!shell.locked(), "a tap is Tab, for completion");
        shell.key(Key::Lock);
        frames(&mut shell, 30, FRAME);
        assert!(shell.lock.engaged());
        type_code(&mut shell, "2468");
        assert_eq!(shell.top_title(), "Terminal", "back where it was");
        // From StandBy on the charger, and from deep standby. (The Terminal
        // keeps Esc for itself: held, it leaves.)
        shell.key(Key::Leave);
        frames(&mut shell, 60, FRAME);
        assert_eq!(shell.stack.len(), 1);
        shell.pretend_charger(Some(true));
        for _ in 0..40 {
            shell.tick(1.0);
        }
        assert!(shell.faces_showing() && shell.locked(), "StandBy locks with a passcode");
        shell.key(Key::Lock);
        frames(&mut shell, 30, FRAME);
        assert!(!shell.faces_showing() && shell.lock.engaged() && shell.lock.visible(), "StandBy gives way to the lock screen");
        let mut shell = with_passcode("2468");
        type_code(&mut shell, "2468");
        shell.settings.screen_off = 30;
        shell.settings.battery.deep_after = 10;
        until_deep(&mut shell, 700).expect("deep standby");
        shell.key(Key::Lock);
        frames(&mut shell, 30, FRAME);
        assert_eq!((shell.deep_asleep(), shell.lock.engaged(), shell.lock.visible(), shell.standby_state()), (false, true, true, State::Awake));
    }

    #[test]
    fn the_hold_cue_fills_while_tab_is_held_and_draws_back_if_let_go() {
        let mut shell = shell();
        let now = std::time::Instant::now();
        crate::platform::set_tab_down(Some(now - std::time::Duration::from_millis(100)));
        frames(&mut shell, 1, FRAME);
        assert_eq!(shell.hold_cue.value(), 0.0, "not before the tap's time is past");
        crate::platform::set_tab_down(Some(now - std::time::Duration::from_millis(375)));
        frames(&mut shell, 1, FRAME);
        assert!((shell.hold_cue.value() - 0.5).abs() < 0.05, "half way at 0.375 s: {}", shell.hold_cue.value());
        assert!(shell.animating(), "it moves on the frame beat while held");
        crate::platform::set_tab_down(None);
        frames(&mut shell, 3, FRAME);
        assert!(shell.hold_cue.value() > 0.1, "it draws back, not off at once");
        frames(&mut shell, 60, FRAME);
        assert_eq!(shell.hold_cue.value(), 0.0);
        assert!(!shell.lock.covering(), "let go early: no lock");
    }

    #[test]
    fn musics_keys_work_on_the_lock_screen_without_unlocking() {
        use std::sync::atomic::{AtomicU8, Ordering};
        static ASKED: AtomicU8 = AtomicU8::new(0);
        crate::nowplaying::on_control(|c| {
            ASKED.store(
                match c {
                    crate::nowplaying::Control::PlayPause => 1,
                    crate::nowplaying::Control::Next => 2,
                    crate::nowplaying::Control::Previous => 3,
                },
                Ordering::Relaxed,
            )
        });
        crate::nowplaying::set(Some(crate::nowplaying::NowPlaying {
            title: "Around the World".into(),
            artist: "Daft Punk".into(),
            album: "Homework".into(),
            cover: None,
            position: 10.0,
            duration: 400.0,
            playing: true,
            at: std::time::Instant::now(),
        }));
        let mut shell = with_passcode("2468");
        frames(&mut shell, 70, 1.0 / 30.0);
        assert!(shell.activities.list().iter().any(|a| a.kind == crate::liveactivity::Kind::Music), "music on the lock screen");
        for (media, asked) in [(crate::platform::Media::PlayPause, 1), (crate::platform::Media::Next, 2), (crate::platform::Media::Previous, 3)] {
            shell.key(Key::Media(media));
            assert_eq!(ASKED.load(Ordering::Relaxed), asked);
            assert!(shell.lock.engaged(), "still locked");
        }
        // Without a passcode, shown by hand: music's keys do not lift it.
        let mut open = self::shell();
        open.key(Key::Lock);
        open.key(Key::Media(crate::platform::Media::Next));
        assert!(open.lock.covering());
        crate::nowplaying::set(None);
    }

    #[test]
    fn music_on_the_charger_turns_standby_to_its_face_and_back() {
        let mut shell = shell();
        shell.settings.screen_off = 30;
        shell.pretend_charger(Some(true));
        for _ in 0..40 {
            shell.tick(1.0);
        }
        assert_eq!(shell.faces.shown_face(), crate::lock::Face::Clock);
        crate::nowplaying::set(Some(crate::nowplaying::NowPlaying {
            title: "Da Funk".into(),
            artist: "Daft Punk".into(),
            album: String::new(),
            cover: None,
            position: 0.0,
            duration: 300.0,
            playing: true,
            at: std::time::Instant::now(),
        }));
        frames(&mut shell, 90, 1.0 / 30.0);
        assert_eq!(shell.faces.shown_face(), crate::lock::Face::Music, "picked by itself");
        assert_eq!(shell.settings.lock.face, crate::lock::Face::Clock, "not kept as the face chosen");
        crate::nowplaying::set(None);
        frames(&mut shell, 90, 1.0 / 30.0);
        assert_eq!(shell.faces.shown_face(), crate::lock::Face::Clock, "back once it stops");
    }

    #[test]
    fn a_song_playing_leaves_the_time_and_the_battery_in_view() {
        crate::nowplaying::set(Some(crate::nowplaying::NowPlaying {
            title: "A song with a name long enough to fill the whole island".into(),
            artist: "Somebody".into(),
            album: String::new(),
            cover: None,
            position: 10.0,
            duration: 400.0,
            playing: true,
            at: std::time::Instant::now(),
        }));
        let mut shell = shell();
        frames(&mut shell, 60 * 6, FRAME);
        let music = shell.activities.list().iter().find(|a| a.kind == crate::liveactivity::Kind::Music).cloned().expect("music");
        assert!(shell.activities.opened().is_none(), "no card open");
        // Compact, music is its cover and its level: the song's name is on its card.
        assert!(music.short.is_empty(), "{:?}", music.short);
        // Whatever carries the island (the demo's queue, by the clock, may
        // outrank music), the time and the battery stay in view.
        let island = shell.places.island.expect("the island");
        assert_eq!(shell.status_shown.target(), 1.0, "the time and the battery in view");
        assert!(island.right() + 2.0 <= shell.status_left, "clear of them: {} and {}", island.right(), shell.status_left);
        crate::nowplaying::set(None);
    }

    #[test]
    fn musics_bars_and_sleep_timer_keep_the_compact_island_clear_of_the_time_and_battery_at_every_size() {
        use crate::gfx::TextSize;
        let np = crate::nowplaying::NowPlaying {
            title: "Harbour Lights".into(),
            artist: "Wren & the Tide".into(),
            album: "Low Water".into(),
            cover: None,
            position: 39.0,
            duration: 214.0,
            playing: true,
            at: std::time::Instant::now(),
        };
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            let mut shell = shell();
            shell.settings.text = size;
            shell.apply_settings();
            frames(&mut shell, 10, FRAME);
            // Compact, music is its cover and its bars: no wider than before.
            let mut music = crate::liveactivity::music(Some(&np)).expect("music");
            assert!(crate::liveactivity::compact_w(&mut shell.gfx, &music) <= 70.0, "{size:?}");
            // With the sleep timer, its countdown too, and still clear.
            crate::liveactivity::with_sleep(&mut music, Some(12.0 * 60.0 + 34.0));
            assert_eq!(music.short, "12:34");
            shell.pretend_activities(vec![music.clone()]);
            frames(&mut shell, 60 * 5, FRAME);
            let island = shell.places.island.expect("the island");
            assert!(shell.activities.opened().is_none() || shell.activities.open.value() < 0.01, "closed again");
            assert_eq!(shell.status_shown.target(), 1.0, "the time and the battery in view at {size:?}");
            assert!(island.right() + 2.0 <= shell.status_left, "clear of them at {size:?}: {} and {}", island.right(), shell.status_left);
            assert!(island.w >= crate::liveactivity::compact_w(&mut shell.gfx, &music) - 1.0, "the countdown whole at {size:?}");
            // Shown and playing, the bars move from frame to frame.
            let mut seen = Vec::new();
            for _ in 0..30 {
                frames(&mut shell, 1, FRAME);
                seen.push(shell.activities.bars.heights());
            }
            assert!(seen.windows(2).filter(|w| w[0] != w[1]).count() > 20, "the bars dance at {size:?}");
            // Ambient, not motion: a few frames a second of their own, and the
            // frame beat free to rest (as motion they held every page at 60).
            assert!(!shell.moving(), "not motion");
            assert!(shell.ambient().is_some(), "frames kept coming while they do");
            // Paused, they settle flat, and stop asking for frames.
            shell.pretend_activities(vec![crate::liveactivity::music(Some(&crate::nowplaying::NowPlaying { playing: false, ..np.clone() })).expect("music")]);
            frames(&mut shell, 60 * 6, FRAME);
            assert!(shell.activities.bars.heights().iter().all(|h| *h < 0.01), "flat at {size:?}");
            assert!(!shell.activities.bars.tick(FRAME, FRAME, false, 0), "still");
            assert!(shell.ambient().is_none(), "and they ask for no frames");
        }
    }

    /// The lock screen's robot turning on its stand (depth.rs) is ambient,
    /// as Music's bars are: a frame every `DRIFT_EVERY` (12.5 a second),
    /// the frame beat at rest; still (no frames) in the battery saver, in
    /// the calm version, under StandBy, and with the depth effect off.
    #[test]
    fn the_lock_screens_robot_turns_a_few_times_a_second_and_is_not_motion() {
        // Its own minute, so the clock turning never draws a frame meanwhile.
        crate::faces::pretend_time(chrono::NaiveDate::from_ymd_opt(2026, 11, 21).and_then(|d| d.and_hms_opt(9, 41, 0)));
        let mut shell = shell();
        shell.settings.lock.follows = false;
        shell.apply_settings();
        // No live activity (the demo robot's queue would bring the time up top).
        shell.pretend_activities(Vec::new());
        shell.lock_as_if_held();
        frames(&mut shell, 120, FRAME);
        assert!(shell.lock.covering());
        assert!(shell.lock.drifting(), "idle, it turns");
        assert_eq!(shell.ambient(), Some(crate::depth::DRIFT_EVERY), "12.5 frames a second");
        assert!(!shell.moving() && !shell.animating(), "the frame beat rests");
        let dt = crate::depth::DRIFT_EVERY.as_secs_f32();
        for _ in 0..10 {
            assert!(shell.tick(dt), "each look draws the turn");
            shell.draw();
            assert!(!shell.moving(), "and it is never motion");
        }
        // Held still: no frames of its own.
        for (name, set) in [("saver", 0), ("calm", 1), ("depth off", 2)] {
            match set {
                0 => shell.settings.battery.saver = true,
                1 => shell.settings.calm = true,
                _ => shell.settings.lock.depth = false,
            }
            frames(&mut shell, 3, dt);
            assert!(!shell.lock.drifting(), "{name}");
            assert_eq!(shell.ambient(), None, "{name}");
            assert!(!shell.tick(dt), "{name}: nothing to draw");
            shell.settings.battery.saver = false;
            shell.settings.calm = false;
            shell.settings.lock.depth = true;
            frames(&mut shell, 3, dt);
            assert_eq!(shell.ambient(), Some(crate::depth::DRIFT_EVERY), "{name}: turning again");
        }
        crate::faces::pretend_time(None);
    }

    #[test]
    fn a_live_activity_carries_the_island_and_a_change_opens_it_for_a_moment() {
        let mut shell = shell();
        shell.pretend_activities(crate::liveactivity::timers(Some((200.0, 300.0, 1)), None));
        frames(&mut shell, 60 * 5, FRAME);
        let island = shell.places.island.expect("the island");
        shell.pretend_activities(crate::liveactivity::timers(Some((40.0, 300.0, 1)), None));
        frames(&mut shell, 60, FRAME);
        let open = shell.places.island.expect("the island");
        assert!(open.h > island.h * 2.0 && open.w > island.w, "opened on the timer's last minute");
        frames(&mut shell, 60 * 5, FRAME);
        let closed = shell.places.island.expect("the island");
        assert!((closed.h - island.h).abs() < 0.5, "closed again");
        // Something said outranks it, and it comes back after.
        shell.say("battery at 15%");
        frames(&mut shell, 2, FRAME);
        assert_eq!(shell.island_words, "battery at 15%");
    }

    fn note(app: &str, title: &str) -> crate::notify::Notification {
        crate::notify::Notification::new(app, "notifications", title)
    }

    #[test]
    fn a_notification_is_kept_said_and_counted_in_the_island_at_rest() {
        let mut shell = shell();
        // Over a live activity (the demo's next match), it is said too.
        frames(&mut shell, 120, FRAME);
        assert!(!shell.activities.list().is_empty());
        shell.notify(note("Catalogue", "Scouting installed"));
        frames(&mut shell, 40, FRAME);
        assert_eq!(shell.island_words, "Scouting installed", "said over the activity");
        frames(&mut shell, 60 * 4, FRAME);
        shell.notes.clear();
        // No live activity carrying the island: it is at rest.
        shell.pretend_activities(Vec::new());
        frames(&mut shell, 200, FRAME);
        let rest = shell.places.island.expect("the island").w;
        shell.notify(note("Music", "12 songs saved").body("in the library"));
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("12 songs saved · in the library"));
        frames(&mut shell, 2, FRAME);
        assert_eq!(shell.island_words, "12 songs saved · in the library", "said in the island");
        // Said and gone; kept, and counted at the island's end.
        frames(&mut shell, 60 * 4, FRAME);
        assert!(shell.message.is_none());
        assert_eq!(shell.notes.unread(), 1);
        let counted = shell.places.island.expect("the island").w;
        assert!(counted > rest + 4.0, "the count widens the island at rest: {rest} → {counted}");
        // Seen in the notification centre: the count goes.
        shell.key(Key::Ctrl(crate::platform::NOTIFICATIONS));
        shell.key(Key::Back);
        frames(&mut shell, 120, FRAME);
        assert_eq!(shell.notes.unread(), 0);
        assert!((shell.places.island.expect("the island").w - rest).abs() < 1.0);
    }

    #[test]
    fn quiet_keeps_them_unsaid_and_dark_but_reminders_timers_and_queue_now_break_through() {
        use crate::lights::Mode;
        let mut shell = shell();
        shell.notes.set_quiet(true);
        shell.notify(note("Music", "12 songs saved"));
        assert!(shell.message.is_none(), "unsaid");
        assert_eq!(shell.notes.unread(), 1, "still kept");
        shell.notify(note("Clock", "timer 5 min done").urgent());
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("timer 5 min done"), "it breaks through");
        // With the screen off: Fn blinks for one that is said, not for a quiet one.
        shell.settings.screen_off = 30;
        until_off(&mut shell, 60).expect("the screen goes off");
        shell.notify(note("Catalogue", "Scouting installed"));
        assert!(!shell.lights.showing().contains(&Some(Mode::Blink)), "Quiet: the lights stay dark");
        shell.notes.set_quiet(false);
        shell.notify(note("Catalogue", "Scouting installed"));
        assert_eq!(shell.lights.showing()[1], Some(Mode::Blink), "Fn blinks until it wakes");
    }

    #[test]
    fn one_from_the_app_in_front_is_seen_there_and_not_said() {
        let mut shell = shell();
        open_app(&mut shell, "Robot", "Logs");
        shell.message = None;
        shell.notify(note("Logs", "how Q30 went"));
        assert!(shell.message.is_none() && shell.notes.unread() == 0 && shell.notes.list().len() == 1);
        // Clock names itself by its page; a notification knows it as Clock.
        let clock = crate::clock::Clock::new();
        assert_eq!((clock.title(), clock.app()), ("Stopwatch", "Clock"));
    }

    #[test]
    fn ctrl_backspace_brings_them_down_and_enter_goes_where_one_leads() {
        let mut shell = shell();
        shell.notify(note("Robot", "brownout").action("builtin:Logs"));
        let id = shell.notes.list()[0].id;
        shell.key(Key::Ctrl(crate::platform::NOTIFICATIONS));
        frames(&mut shell, 40, FRAME);
        assert!(shell.control_open() && shell.control.page() == crate::control::Page::Notices);
        assert_eq!(shell.notes.unread(), 0, "seen");
        // Enter: the sheet goes, where it leads opens, and it is put away.
        shell.key(Key::Enter);
        frames(&mut shell, 90, FRAME);
        assert!(!shell.control.visible());
        assert_eq!(shell.top_title(), "Logs");
        assert!(shell.notes.list().iter().all(|n| n.id != id));
        // Ctrl-Backspace again puts it away; from the control centre it turns to them.
        shell.key(Key::Ctrl(crate::platform::NOTIFICATIONS));
        frames(&mut shell, 30, FRAME);
        shell.key(Key::Ctrl(crate::platform::NOTIFICATIONS));
        assert!(!shell.control_open());
        frames(&mut shell, 60, FRAME);
        shell.key(Key::Ctrl(' '));
        shell.key(Key::Char('n'));
        assert!(shell.control_open() && shell.control.page() == crate::control::Page::Notices);
        shell.key(Key::Back);
        assert!(!shell.control_open());
    }

    #[test]
    fn posted_from_another_thread_it_is_taken_in_on_the_next_turn() {
        let mut shell = shell();
        crate::notify::post(note("Assistant", "your answer is written").action("builtin:Assistant"));
        assert!(shell.notes.list().is_empty());
        frames(&mut shell, 1, FRAME);
        assert_eq!(shell.notes.list().len(), 1);
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("your answer is written"));
    }

    #[test]
    fn locked_they_show_on_the_lock_screen_and_one_picked_opens_once_unlocked() {
        let mut shell = with_passcode("2468");
        shell.pretend_activities(Vec::new());
        shell.notify(note("Clock", "timer 5 min done").urgent().action("builtin:Logs"));
        shell.notify(note("Music", "12 songs saved").action("builtin:Logs"));
        frames(&mut shell, 30, FRAME);
        assert_eq!(shell.lock_info().notes.len(), 2, "both, unseen");
        // ↓ picks the newest; the passcode opens the lock and then it.
        shell.key(Key::Down);
        frames(&mut shell, 2, FRAME);
        assert!(shell.lock.pick().is_some());
        let picked = shell.lock.pick().unwrap();
        type_code(&mut shell, "2468");
        assert!(!shell.locked());
        frames(&mut shell, 60, FRAME);
        let opened = if shell.notes.list().iter().any(|n| n.id == picked) { None } else { Some(()) };
        assert!(opened.is_some(), "put away as it opened");
        assert_eq!(shell.top_title(), "Logs");
        // What came while it was locked is said as it opens.
        assert!(shell.message.is_some() || shell.notes.list().len() == 1);
    }

    #[test]
    fn a_passcode_taken_off_over_ssh_leaves_no_lock_with_no_way_in() {
        let mut shell = with_passcode("2468");
        assert!(shell.locked());
        std::fs::remove_file(crate::passcode::file()).unwrap();
        shell.key(Key::Char('1'));
        shell.key(Key::Enter);
        frames(&mut shell, 20, FRAME);
        assert!(!shell.locked());
    }

    #[test]
    fn deep_standby_comes_after_the_wait_and_its_steps_are_undone_in_reverse() {
        use crate::deep::STEPS;
        let mut shell = shell();
        shell.settings.screen_off = 30;
        shell.settings.battery.deep_after = 10;
        // Ten minutes with no key (the shell is a few seconds old already).
        let at = until_deep(&mut shell, 700).expect("deep standby");
        assert!((590..=605).contains(&at), "deep after {at} s");
        assert!(shell.panel_off() && !shell.animating());
        assert_eq!(shell.deep.log, STEPS.map(|s| (s, true)), "pollers, motion, lights, system, slack, in that order");
        // A minute at a time, and nothing drawn.
        assert!(!shell.tick(60.0));
        assert!(shell.deep_asleep());
        // A key: awake, and every step undone, the last first.
        shell.key(Key::Down);
        assert!(!shell.deep_asleep());
        assert_eq!(shell.standby_state(), crate::standby::State::Awake);
        let mut back: Vec<_> = STEPS.map(|s| (s, false)).to_vec();
        back.reverse();
        assert_eq!(shell.deep.log[STEPS.len()..], back[..]);
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("slept 1 m"), "too short to tell its draw");
        // The wait starts again from the key.
        assert_eq!(until_deep(&mut shell, 300), None);
    }

    #[test]
    fn the_key_that_wakes_deep_standby_reaches_no_app_nor_do_its_repeats() {
        let mut shell = shell();
        shell.settings.screen_off = 30;
        shell.settings.battery.deep_after = 10;
        until_deep(&mut shell, 700).expect("deep standby");
        // Enter on Home's first tile: it wakes the handheld, and opens nothing.
        shell.key(Key::Enter);
        assert!(!shell.deep_asleep());
        // Held, it repeats (the first repeat 0.4 s in, then quicker).
        for gap in [0.4, 0.1, 0.1, 0.1] {
            frames(&mut shell, 1, gap);
            shell.key(Key::Enter);
        }
        frames(&mut shell, 40, FRAME);
        assert_eq!(shell.stack.len(), 1, "neither the waking key nor its repeats opened the tile");
        // Let go and pressed again: a key like any other.
        frames(&mut shell, 40, FRAME);
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        assert_eq!(shell.stack.len(), 2, "Enter opens the tile now");
        // Another key straight after waking is not the waking key's repeat.
        let mut shell = self::shell();
        shell.settings.screen_off = 30;
        shell.settings.battery.deep_after = 10;
        until_deep(&mut shell, 700).expect("deep standby");
        shell.key(Key::Right);
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        assert_eq!(shell.stack.len(), 2, "a different key goes through");
    }

    #[test]
    fn what_would_be_cut_off_keeps_deep_standby_away() {
        let mut shell = shell();
        shell.settings.screen_off = 30;
        // Never: the screen off, and nothing more.
        shell.settings.battery.deep_after = 0;
        assert_eq!(until_deep(&mut shell, 2 * 3600), None);
        // A Clock timer running: it must be said when it runs out.
        shell.settings.battery.deep_after = 10;
        TIMER_RUNNING.with(|t| t.set(true));
        assert_eq!(until_deep(&mut shell, 1200), None);
        TIMER_RUNNING.with(|t| t.set(false));
        let at = until_deep(&mut shell, 60).expect("deep once the timer is done");
        assert!(at <= 11, "asked again within ten seconds: {at}");
        shell.key(Key::Back);
    }

    #[test]
    fn waking_says_how_long_it_slept_and_what_it_drew() {
        use crate::power::{Flow, Reading};
        let mut shell = shell();
        let night = Reading { percent: 41, flow: Flow::Discharging, milliamps: Some(180.0), volts: Some(3.8), ..Reading::default() };
        shell.deep_for(7.0 * 3600.0 + 12.0 * 60.0, night.clone());
        shell.key(Key::Char('a'));
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("slept 7 h 12 m · ~0.68 W"));
        assert_eq!(shell.stack.len(), 1, "the waking key typed nothing anywhere");
        // Another night, drawing more; then on the charger, no draw to give.
        shell.deep_for(2.0 * 3600.0, Reading { percent: 9, milliamps: Some(250.0), ..night.clone() });
        shell.key(Key::Char('b'));
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("slept 2 h 0 m · ~0.95 W"));
        shell.deep_for(3600.0, Reading { flow: Flow::Charging, ..night });
        shell.key(Key::Char('c'));
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("slept 1 h 0 m · charging"));
    }

    // ------------------------------------------------------- music standby ---

    /// A song playing (or paused), as Music tells the live activities.
    fn music(playing: bool) {
        crate::nowplaying::set(Some(crate::nowplaying::NowPlaying {
            title: "Slow Signal".into(),
            artist: "Mara Ellis".into(),
            album: "Night Bus".into(),
            cover: None,
            position: 0.0,
            duration: 201.0,
            playing,
            at: std::time::Instant::now(),
        }));
    }

    /// A shell playing music, its screen off after half a minute, in music
    /// standby: returns the seconds it took.
    fn into_music_standby(shell: &mut Shell) -> u32 {
        shell.settings.screen_off = 30;
        music(true);
        let at = until_deep(shell, 120).expect("music standby");
        assert!(shell.deep.music());
        at
    }

    #[test]
    fn music_standby_comes_half_a_minute_into_the_dark_and_keeps_the_sound() {
        use crate::deep::STEPS;
        let mut shell = shell();
        let at = into_music_standby(&mut shell);
        // Dark at about 31 s, then half a minute (the shell is two seconds old).
        assert!((55..=64).contains(&at), "music standby after {at} s");
        assert!(shell.panel_off() && !shell.animating());
        assert_eq!(shell.deep.log, STEPS.map(|s| (s, true)), "deep standby's steps, in order");
        assert_eq!(shell.deep.asked(), [vec!["deep", "on", "keep-wifi", "keep-radios"]], "the radios kept, and Wi-Fi: it streams");
        // A minute at a time, nothing drawn, still playing.
        assert!(!shell.tick(60.0));
        assert!(shell.deep.music());
        // The sound keys do their work without waking it. (Play and next go
        // to the one player every test shares, so they are not pressed
        // here; they take the same way through `Shell::key`.)
        {
            let _sound = crate::sound::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let before = crate::sound::level();
            for media in [crate::platform::Media::VolumeUp, crate::platform::Media::VolumeDown] {
                shell.key(Key::Media(media));
                assert!(shell.deep.music(), "{media:?} keeps it asleep");
            }
            crate::sound::set_level(before);
        }
        // Any other key wakes it, every step undone the last first, and
        // nothing said (the lock screen shows the music).
        shell.message = None;
        shell.key(Key::Down);
        assert!(!shell.deep_asleep());
        assert_eq!(shell.standby_state(), crate::standby::State::Awake);
        let mut back: Vec<_> = STEPS.map(|s| (s, false)).to_vec();
        back.reverse();
        assert_eq!(shell.deep.log[STEPS.len()..], back[..]);
        assert_eq!(shell.deep.asked().last().unwrap(), &vec!["deep", "off"]);
        assert_eq!(shell.message, None);
    }

    #[test]
    fn music_standby_lets_wifi_go_once_what_plays_next_is_here() {
        let mut shell = shell();
        crate::nowplaying::on_needs_network(|| false);
        into_music_standby(&mut shell);
        assert_eq!(shell.deep.asked(), [vec!["deep", "on", "keep-radios"]], "played from the library: Wi-Fi off too");
        // The queue goes back to streaming: out (the screen stays dark),
        // and in again with Wi-Fi.
        crate::nowplaying::on_needs_network(|| true);
        shell.tick(60.0);
        assert!(!shell.deep_asleep() && shell.panel_off());
        assert_eq!(shell.deep.asked().last().unwrap(), &vec!["deep", "off"]);
        until_deep(&mut shell, 60).expect("music standby again");
        assert_eq!(shell.deep.asked().last().unwrap(), &vec!["deep", "on", "keep-wifi", "keep-radios"]);
    }

    #[test]
    fn music_stopped_moves_on_to_standby_and_then_deep_standby() {
        let mut shell = shell();
        shell.settings.battery.deep_after = 10;
        into_music_standby(&mut shell);
        // Paused after a few minutes: it waits as long as the screen takes to
        // go off, then gives way to plain standby (deep standby's ten
        // minutes are not up), the screen still dark.
        shell.tick(60.0);
        shell.tick(60.0);
        music(false);
        shell.tick(20.0);
        assert!(shell.deep.music(), "a moment's pause");
        shell.tick(20.0);
        assert!(!shell.deep_asleep() && shell.panel_off(), "plain standby, dark");
        assert_eq!(shell.deep.asked().last().unwrap(), &vec!["deep", "off"]);
        // Deep standby's wait counts the music's time: about four minutes
        // already, so the rest of the ten, not ten more.
        let at = until_deep(&mut shell, 600).expect("deep standby");
        assert!(at < 400, "deep after {at} s more");
        assert!(!shell.deep.music());
        assert_eq!(shell.deep.asked().last().unwrap(), &vec!["deep", "on"]);
    }

    #[test]
    fn music_stopped_after_a_long_listen_goes_deep_in_place() {
        let mut shell = shell();
        shell.settings.battery.deep_after = 10;
        into_music_standby(&mut shell);
        // Half an hour of music, then it ends.
        for _ in 0..30 {
            shell.tick(60.0);
        }
        music(false);
        shell.tick(60.0);
        assert!(shell.deep_asleep() && !shell.deep.music(), "deep standby, never out of it");
        assert_eq!(
            shell.deep.asked(),
            [vec!["deep", "on", "keep-wifi", "keep-radios"], vec!["deep", "on"]],
            "Wi-Fi and the radios go too; nothing else is taken twice"
        );
        // A key wakes it, and the island says deep standby's own time.
        shell.key(Key::Down);
        assert!(shell.message.as_ref().is_some_and(|(m, _)| m.starts_with("slept ")), "{:?}", shell.message);
    }

    #[test]
    fn what_music_standby_would_cut_off_keeps_it_away() {
        let mut shell = shell();
        shell.settings.screen_off = 30;
        music(true);
        TIMER_RUNNING.with(|t| t.set(true));
        assert_eq!(until_deep(&mut shell, 300), None, "a Clock timer running: it must be said");
        TIMER_RUNNING.with(|t| t.set(false));
        assert!(until_deep(&mut shell, 11).is_some(), "the timer done: within ten seconds");
        // Nothing playing: no music standby, and deep standby as ever.
        let mut shell = self::shell();
        shell.settings.screen_off = 30;
        music(false);
        assert_eq!(until_deep(&mut shell, 300), None);
    }

    /// A shell with an empty book of reminders (each test's thread has its own).
    fn reminding() -> Shell {
        crate::reminders::with(|s| s.book = crate::reminders::Book::memory());
        shell()
    }

    #[test]
    fn a_reminder_due_with_the_screen_off_wakes_it_and_opens_reminders() {
        use crate::lights::Mode;
        let mut shell = reminding();
        shell.settings.screen_off = 30;
        crate::reminders::remind("swap battery in 1 min");
        for _ in 0..40 {
            shell.tick(1.0);
        }
        assert_eq!(shell.standby_state(), crate::standby::State::Off);
        assert_eq!(shell.lights.showing(), [None; 3], "nothing yet");
        crate::reminders::later(60);
        shell.tick(0.25);
        assert_eq!(shell.standby_state(), crate::standby::State::Awake);
        assert_eq!(says(&shell), "swap battery");
        assert_eq!(shell.lights.showing(), [Some(Mode::Fast); 3], "the keyboard's lights blink");
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Reminders", "opened on what is due");
        assert!(shell.message.is_some(), "held longer than a message");
    }

    #[test]
    fn a_note_coming_due_opens_the_note_over_reminders() {
        use crate::reminders::{parse::Parsed, plan, Link, Repeat, When};
        let mut shell = reminding();
        shell.settings.screen_off = 30;
        let file = "reminded-note.txt";
        std::fs::create_dir_all(crate::notes::dir()).unwrap();
        std::fs::write(crate::notes::dir().join(file), "Elevator belt\nretension the follower\n").unwrap();
        crate::reminders::with(|s| {
            let n = plan::Now { at: crate::reminders::now(), zone: &plan::Here, view: None };
            s.book.add(Parsed { text: "Elevator belt".into(), when: When::In(n.at + 60), repeat: Repeat::Never, list: None, link: Link::Note(file.into()) }, &n);
        });
        for _ in 0..40 {
            shell.tick(1.0);
        }
        crate::reminders::later(60);
        frames(&mut shell, 90, FRAME);
        let titles: Vec<String> = shell.stack.iter().map(|(s, _)| s.title().to_owned()).collect();
        assert_eq!(titles.len(), 3, "Home, Reminders and the note: {titles:?}");
        assert_eq!(titles[1], "Reminders");
        // Out of the note (Ctrl-Q, as the editor is left): Reminders, on it, to tick it off.
        shell.key(Key::Ctrl('q'));
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Reminders");
    }

    #[test]
    fn on_a_screen_in_use_a_reminder_is_only_said() {
        let mut shell = reminding();
        crate::reminders::remind("check bumpers in 1 min");
        frames(&mut shell, 10, FRAME);
        crate::reminders::later(61);
        shell.key(Key::Right);
        frames(&mut shell, 60, FRAME);
        assert_eq!(says(&shell), "check bumpers");
        assert_eq!(shell.stack.len(), 1, "Home stays: whoever is using it is not interrupted");
    }

    #[test]
    fn in_deep_standby_a_reminder_wakes_it_on_time_with_no_wakeup_added() {
        let mut shell = reminding();
        shell.settings.screen_off = 30;
        shell.settings.battery.deep_after = 10;
        crate::reminders::remind("check bumpers in 20 min");
        until_deep(&mut shell, 700).expect("deep standby");
        let due = shell.reminders.due().expect("worked out before it went deep");
        // The minute's wait as ever, until the reminder is less than a minute off.
        assert_eq!(shell.deep_wait(), crate::deep::WAIT);
        crate::reminders::later(due - crate::reminders::now() - 25);
        let wait = shell.deep_wait().as_secs();
        assert!((24..=25).contains(&wait), "the wait ends as it comes due: {wait} s");
        // That wait's tick, a moment early: still deep.
        assert!(!shell.tick(20.0));
        assert!(shell.deep_asleep());
        crate::reminders::later(25);
        shell.tick(5.0);
        assert!(!shell.deep_asleep(), "woken");
        assert_eq!(says(&shell), "check bumpers", "the reminder, not how long it slept");
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Reminders");
    }

    #[test]
    fn deep_standby_wakes_for_the_time_to_queue() {
        let mut shell = reminding();
        shell.settings.screen_off = 30;
        shell.settings.battery.deep_after = 10;
        until_deep(&mut shell, 700).expect("deep standby");
        shell.queue_due = Some(crate::reminders::now() + 90);
        assert!(shell.deep_wait() <= crate::deep::WAIT);
        shell.tick(60.0);
        assert!(shell.deep_asleep(), "not yet");
        crate::reminders::later(90);
        shell.tick(30.0);
        assert!(!shell.deep_asleep());
        assert_eq!(shell.standby_state(), crate::standby::State::Awake);
    }

    #[test]
    fn the_queue_alert_is_worked_out_ahead_for_each_match() {
        use crate::schedule::{Match, View};
        let t = 1_800_000_000;
        let m = |label: &str, at: i64| Match { key: label.to_lowercase(), label: label.into(), time: Some(at), queue_at: Some(at - 900), ..Match::default() };
        let mut view = View { team: 5805, matches: vec![m("Q31", t), m("Q40", t + 3000)], ..View::default() };
        let now = t - 2000;
        assert_eq!(queue_alert_at(&view, now, &[]), Some(t - 900 - QUEUE_AHEAD), "five minutes before queueing for Q31");
        assert_eq!(queue_alert_at(&view, now, &[("q31".into(), 0)]), Some(t - 900), "then the time itself");
        // Both said for Q31: Q40's warning, once Q31 is no longer the next.
        assert_eq!(queue_alert_at(&view, now, &[("q31".into(), 1)]), Some(t + 2100 - QUEUE_AHEAD));
        // Already in its window: due now.
        assert_eq!(queue_alert_at(&view, t - 1000, &[]), Some(t - 1000));
        view.matches[1].queue_at = None;
        assert_eq!(queue_alert_at(&view, now, &[("q31".into(), 1)]), None, "no queue time: nothing to work out");
    }

    /// A plain page for pushing.
    struct Page(&'static str);

    impl Screen for Page {
        fn title(&self) -> &str {
            self.0
        }
        fn key(&mut self, _key: Key, _cx: &mut Cx) -> Nav {
            Nav::Stay
        }
        fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
            let ink = g.tokens.ink;
            g.surface(Rect::new(PAD, 40.0, W - 2.0 * PAD, 40.0), 8.0, ink);
        }
    }

    #[test]
    fn a_page_slides_in_runs_a_little_past_and_settles_exactly() {
        let mut shell = shell();
        shell.open(Box::new(Page("one")), Path::Slide);
        let mut most = 0.0f32;
        let mut n = 0;
        while let Some(p) = shell.progress() {
            most = most.max(p);
            frames(&mut shell, 1, 0.025);
            n += 1;
            assert!(n < 60, "it never came to rest");
        }
        // A few pixels past its place (the panel is 320 wide), then home.
        assert!(most > 1.005 && most < 1.03, "furthest {most}");
        assert!(n as f32 * 0.025 < 0.9, "{n} frames");
    }

    #[test]
    fn a_page_pushed_while_the_last_is_still_arriving_carries_that_one_on() {
        let mut shell = shell();
        shell.open(Box::new(Page("one")), Path::Slide);
        frames(&mut shell, 4, 0.025);
        let t = shell.transition.as_ref().unwrap();
        let (p, v) = (t.progress.value(), t.progress.velocity());
        assert!(p > 0.2 && p < 0.9 && v > 0.5, "mid-way and moving: {p} at {v}/s");
        // Enter again before it lands: its slide goes on beneath the new page,
        // from where it was and at its speed, rather than landing at once.
        let picture_before = shell.gfx.canvas.px.clone();
        shell.open(Box::new(Page("two")), Path::Slide);
        let beneath = &shell.transition.as_ref().unwrap().beneath.as_ref().expect("carried on").progress;
        assert_eq!((beneath.value(), beneath.velocity()), (p, v));
        // Its first frame: the new page has not moved yet, so nothing else has either.
        frames(&mut shell, 1, 0.0);
        let same = picture_before.iter().zip(&shell.gfx.canvas.px).filter(|(a, b)| a == b).count();
        assert!(same as f32 > 0.97 * picture_before.len() as f32, "no jump: {same} of {} pixels the same", picture_before.len());
        let mut n = 0;
        while shell.transition.is_some() {
            frames(&mut shell, 1, 0.025);
            n += 1;
            assert!(n < 60, "it never came to rest");
        }
        assert_eq!(shell.stack.len(), 3);
        // A page pushed from rest has nothing beneath it.
        shell.open(Box::new(Page("three")), Path::Slide);
        assert!(shell.transition.as_ref().unwrap().beneath.is_none());
    }

    // ----------------------------------------------------------- spotlight ---

    /// Spotlight's index read in full now, rather than whenever its thread
    /// gets there.
    fn index_read() {
        crate::spotlight::index().refresh(true);
    }

    fn type_in(shell: &mut Shell, text: &str) {
        for c in text.chars() {
            shell.key(Key::Char(c));
            frames(shell, 1, FRAME);
        }
    }

    /// The stack's titles, Home first.
    fn titles_of(shell: &Shell) -> Vec<String> {
        shell.stack.iter().map(|(s, _)| s.title().to_owned()).collect()
    }

    /// Ctrl-Enter, some typing, Esc: from wherever it was, back to there,
    /// with nothing typed reaching the screen behind.
    fn up_and_away(shell: &mut Shell, typed: &str) {
        let before = titles_of(shell);
        shell.key(Key::Ctrl('\n'));
        assert!(shell.spotlight_open(), "over {before:?}");
        type_in(shell, typed);
        assert_eq!(shell.spotlight.query(), typed, "the keys went to Spotlight");
        assert_eq!(titles_of(shell), before, "and nothing reached the screen behind");
        shell.key(Key::Back);
        assert!(!shell.spotlight_open());
        frames(shell, 90, FRAME);
        assert!(!shell.spotlight.visible() && shell.spotlight_under.is_none(), "gone, and its picture let go");
        assert_eq!(titles_of(shell), before);
    }

    #[test]
    fn ctrl_enter_brings_spotlight_up_over_any_screen_and_esc_goes_back_there() {
        let mut shell = shell();
        // Home, where the bare letters are arrows: here they type.
        up_and_away(&mut shell, "fxzc");
        // An app.
        open_app(&mut shell, "Robot", "Logs");
        up_and_away(&mut shell, "logs");
        // A page deep in Settings, with the focus moved.
        back_home(&mut shell);
        open_app(&mut shell, "Robot", "Settings");
        shell.key(Key::Down);
        shell.key(Key::Enter);
        frames(&mut shell, 60, FRAME);
        up_and_away(&mut shell, "wi");
        // The terminal, which takes every other key for its program.
        back_home(&mut shell);
        open_app(&mut shell, "Settings", "Terminal");
        up_and_away(&mut shell, "ls");
        // Over the control centre, which gives way to it.
        shell.key(Key::Ctrl(' '));
        frames(&mut shell, 30, FRAME);
        shell.key(Key::Ctrl('\n'));
        assert!(shell.spotlight_open() && !shell.control_open() && !shell.control.visible());
        // Ctrl-Space from Spotlight: the control centre in its place.
        shell.key(Key::Ctrl(' '));
        assert!(!shell.spotlight.visible() && shell.control_open());
        shell.key(Key::Back);
        frames(&mut shell, 60, FRAME);
        assert_eq!(title(&shell), "Terminal");
    }

    #[test]
    fn space_and_slash_on_home_still_search() {
        let mut shell = shell();
        shell.key(Key::Char(' '));
        assert!(shell.spotlight_open(), "Space, as before");
        assert_eq!(shell.spotlight.query(), "", "and is not typed");
        shell.key(Key::Ctrl('q'));
        assert!(!shell.spotlight_open(), "Ctrl-Q puts it away too");
        frames(&mut shell, 90, FRAME);
        shell.key(Key::Char('/'));
        assert!(shell.spotlight_open());
        // Ctrl-Enter again puts it away.
        shell.key(Key::Ctrl('\n'));
        assert!(!shell.spotlight_open());
        assert_eq!(shell.stack.len(), 1);
    }

    #[test]
    fn it_grows_out_of_the_island_its_glass_ramping_and_springs_back_into_it() {
        let mut shell = shell();
        let island = shell.island_rect();
        shell.key(Key::Ctrl('\n'));
        frames(&mut shell, 3, FRAME);
        let bar = shell.spotlight.glass(&shell.gfx, island);
        assert!(bar.strength > 0.0 && bar.strength < 1.0 || shell.spotlight.shown() < 0.6, "it materializes: {}", bar.strength);
        assert!(bar.rect.w > island.w && bar.rect.w < W, "growing out of the island: {:?}", bar.rect);
        assert!(shell.spotlight_under.is_some(), "the screen behind is held still");
        frames(&mut shell, 60, FRAME);
        let rest = shell.spotlight.glass(&shell.gfx, island);
        assert!((rest.strength - 1.0).abs() < 0.01 && rest.rect.x <= island.x && rest.rect.y == island.y);
        // Esc: it shrinks back into the island; its glass lingers a moment, then goes.
        shell.key(Key::Back);
        frames(&mut shell, 4, FRAME);
        let shrinking = shell.spotlight.glass(&shell.gfx, island);
        assert!(shrinking.rect.w < rest.rect.w && shrinking.strength > 0.99, "{shrinking:?}");
        frames(&mut shell, 30, FRAME);
        assert!(shell.spotlight.glass(&shell.gfx, island).strength < 0.5);
        frames(&mut shell, 60, FRAME);
        assert!(!shell.spotlight.visible());
        assert!(!shell.moving(), "and everything comes to rest");
    }

    #[test]
    fn turned_round_on_its_way_back_it_keeps_what_was_typed() {
        let mut shell = shell();
        shell.key(Key::Ctrl('\n'));
        frames(&mut shell, 30, FRAME);
        type_in(&mut shell, "git");
        shell.key(Key::Back);
        frames(&mut shell, 5, FRAME);
        let part_way = shell.spotlight.shown();
        assert!(part_way > 0.05 && part_way < 0.99, "{part_way}");
        shell.key(Key::Ctrl('\n'));
        frames(&mut shell, 1, FRAME);
        assert!((shell.spotlight.shown() - part_way).abs() < 0.15, "no jump: {part_way} then {}", shell.spotlight.shown());
        assert_eq!(shell.spotlight.query(), "git");
        // Any other key while it goes: gone at once, and the key is the screen's.
        shell.key(Key::Back);
        frames(&mut shell, 3, FRAME);
        shell.key(Key::Right);
        assert!(!shell.spotlight.visible());
    }

    #[test]
    fn a_result_closed_while_still_opening_turns_round_into_its_row() {
        let mut shell = shell();
        index_read();
        shell.key(Key::Ctrl('\n'));
        type_in(&mut shell, "calculatr");
        frames(&mut shell, 30, FRAME);
        shell.key(Key::Enter);
        frames(&mut shell, 3, FRAME);
        assert!(shell.transition.as_ref().is_some_and(|t| t.arriving()), "still growing out of its row");
        // Closed before it has arrived: it turns round from where it is, and
        // no motion is left over Home with nothing to bring in.
        shell.key(Key::Back);
        assert_eq!(shell.stack.len(), 1);
        assert!(shell.transition.as_ref().is_none_or(|t| t.leaving.is_some()), "turned round, not left behind");
        // A redraw from scratch (the text size changed) as it goes.
        shell.settings.text = shell.settings.text.next();
        shell.apply_settings();
        frames(&mut shell, 90, FRAME);
        assert_eq!(title(&shell), "Catalyst");
    }

    #[test]
    fn enter_opens_a_result_out_of_its_row_and_esc_comes_back_to_where_it_was() {
        let mut shell = shell();
        index_read();
        // From an app: a setting, deep-linked to its row.
        open_app(&mut shell, "Robot", "Logs");
        shell.key(Key::Ctrl('\n'));
        frames(&mut shell, 30, FRAME);
        type_in(&mut shell, "brightness");
        frames(&mut shell, 30, FRAME);
        assert_eq!(shell.spotlight.hits().first().map(|h| h.item.title.as_str()), Some("Brightness"));
        shell.key(Key::Enter);
        assert!(!shell.spotlight.visible(), "it becomes what it opens");
        let Some(Transition { path: Path::Grow(from), .. }) = &shell.transition else { panic!("it grows out of its row") };
        assert!(from.y > 20.0 && from.w > 200.0, "the row: {from:?}");
        frames(&mut shell, 90, FRAME);
        assert_eq!(titles_of(&shell), ["Catalyst", "Logs", "Display"]);
        shell.key(Key::Back);
        frames(&mut shell, 90, FRAME);
        assert_eq!(titles_of(&shell), ["Catalyst", "Logs"], "back where Spotlight came up");
        // From Home: an app, by a misspelling.
        back_home(&mut shell);
        shell.key(Key::Ctrl('\n'));
        type_in(&mut shell, "calculatr");
        frames(&mut shell, 30, FRAME);
        shell.key(Key::Enter);
        frames(&mut shell, 90, FRAME);
        assert_eq!(title(&shell), "Calculator");
        shell.key(Key::Back);
        frames(&mut shell, 90, FRAME);
        assert_eq!(title(&shell), "Catalyst");
        // A sum: the calculator, the sum typed in it.
        shell.key(Key::Ctrl('\n'));
        type_in(&mut shell, "12*7");
        assert_eq!(shell.spotlight.hits().first().map(|h| h.item.title.as_str()), Some("= 84"), "worked out at once");
        shell.key(Key::Enter);
        frames(&mut shell, 90, FRAME);
        assert_eq!(title(&shell), "Calculator");
        back_home(&mut shell);
        // An Action: the confirm page first, never straight to running it.
        shell.key(Key::Ctrl('\n'));
        type_in(&mut shell, "disk space");
        shell.key(Key::Enter);
        frames(&mut shell, 90, FRAME);
        assert_eq!(title(&shell), "Confirm");
        shell.key(Key::Back);
        frames(&mut shell, 90, FRAME);
        assert_eq!(title(&shell), "Catalyst", "not run");
        // Asked again, and Enter: it only looks, so "Run it" is first, and the
        // page becomes the command's terminal in place.
        shell.key(Key::Ctrl('\n'));
        type_in(&mut shell, "disk space");
        shell.key(Key::Enter);
        frames(&mut shell, 90, FRAME);
        shell.key(Key::Enter);
        frames(&mut shell, 30, FRAME);
        assert_eq!(titles_of(&shell), ["Catalyst", "Command"]);
        assert!(shell.stack[1].0.wants_text(), "a terminal now");
        shell.key(Key::Ctrl('q'));
        frames(&mut shell, 90, FRAME);
        assert_eq!(title(&shell), "Catalyst");
    }

    #[test]
    fn the_arrows_move_through_the_results_and_tab_previews_one() {
        let mut shell = shell();
        index_read();
        shell.key(Key::Ctrl('\n'));
        frames(&mut shell, 30, FRAME);
        type_in(&mut shell, "volts");
        frames(&mut shell, 30, FRAME);
        let hits: Vec<String> = shell.spotlight.hits().iter().map(|h| h.item.title.clone()).collect();
        assert!(hits.len() >= 3, "{hits:?}");
        assert_eq!(hits.last().map(String::as_str), Some("Ask the Assistant “volts”"), "the assistant last");
        assert!(hits[hits.len() - 2].starts_with("Search the web"), "the web before it");
        // Down and back up, and Tab: a value's preview, from memory at once.
        shell.key(Key::Down);
        shell.key(Key::Up);
        let first = shell.spotlight.hits()[0].clone();
        assert_eq!(first.item.weight, crate::spotlight::weight::VALUE, "{:?}", first.item.title);
        shell.key(Key::Tab);
        assert!(shell.spotlight.previewing());
        frames(&mut shell, 30, FRAME);
        assert!(shell.spotlight.previewing());
        // Left back to the list; typing goes on from there.
        shell.key(Key::Left);
        assert!(!shell.spotlight.previewing());
        shell.key(Key::Backspace);
        assert_eq!(shell.spotlight.query(), "volt");
        // Up past the top stays on the top.
        for _ in 0..4 {
            shell.key(Key::Up);
        }
        frames(&mut shell, 30, FRAME);
        shell.key(Key::Back);
        frames(&mut shell, 90, FRAME);
        assert_eq!(shell.stack.len(), 1);
    }

    /// A screen that locks the handheld, as a lock screen would.
    struct Lock;

    impl Screen for Lock {
        fn title(&self) -> &str {
            "Locked"
        }
        fn key(&mut self, _key: Key, _cx: &mut Cx) -> Nav {
            Nav::Stay
        }
        fn draw(&mut self, _g: &mut Gfx, _robot: &Robot) {}
        fn locks(&self) -> bool {
            true
        }
    }

    #[test]
    fn it_never_opens_over_a_lock_nor_over_setup() {
        let mut shell = shell();
        shell.key(Key::Ctrl('\n'));
        assert!(shell.spotlight_open());
        // Locked while it shows: it goes at once.
        shell.stack.push((Box::new(Lock), Path::Slide));
        frames(&mut shell, 1, FRAME);
        assert!(!shell.spotlight.visible());
        assert!(shell.locked());
        shell.key(Key::Ctrl('\n'));
        assert!(!shell.spotlight_open(), "not over the lock");
        shell.stack.pop();
        assert!(!shell.locked());
        // First-boot setup takes the whole panel: not over it either.
        shell.start_setup();
        frames(&mut shell, 5, FRAME);
        shell.key(Key::Ctrl('\n'));
        assert!(!shell.spotlight_open());
    }

    #[test]
    fn typing_fast_never_waits_for_the_results() {
        let mut shell = shell();
        index_read();
        shell.key(Key::Ctrl('\n'));
        frames(&mut shell, 30, FRAME);
        let mut slowest = std::time::Duration::ZERO;
        for c in "elevator current".chars() {
            let began = std::time::Instant::now();
            shell.key(Key::Char(c));
            slowest = slowest.max(began.elapsed());
            frames(&mut shell, 1, 0.025);
            assert!(!shell.spotlight.hits().is_empty(), "results from the first key");
        }
        // A frame's beat on the handheld is 25 ms; a key here is a sliver of
        // one even on a debug build of a slow computer.
        assert!(slowest < std::time::Duration::from_millis(40), "slowest key {slowest:?}");
        for _ in 0..16 {
            let began = std::time::Instant::now();
            shell.key(Key::Backspace);
            slowest = slowest.max(began.elapsed());
        }
        assert_eq!(shell.spotlight.query(), "");
        assert!(slowest < std::time::Duration::from_millis(40), "slowest key {slowest:?}");
    }
}
