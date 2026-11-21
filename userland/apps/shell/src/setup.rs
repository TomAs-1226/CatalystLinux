//! First start: setting the handheld up, the way a computer's own setup
//! does it. The start animation gives way to a welcome; then, a page at a
//! time, the pages of `FLOW`: how the keyboard works, the team, the
//! handheld's name and time zone, Wi-Fi, keys for online services, a page of
//! choices, and the look; then "just a moment" while it all takes effect,
//! and Home.
//!
//! Setup owns the whole panel (no island, no status), sits on the brand's
//! ground, and every page arrives the same way: its title, its words and
//! its rows rise into place one after another on `release`, from the side
//! the page came from. Esc goes back a page, Tab skips one, and anything
//! skipped can be set later in Settings, or from a computer. The keyboard
//! lesson is also offered on its own, from Settings › Help.
//!
//! Set up from a computer on first start, what it sent arrives a row at a
//! time, and then come the pages only the handheld can do (the hardware
//! check, the keyboard lesson, the name and time zone, power), with those the
//! computer left unanswered.

use bezel::{Accent, Color, Rect, TextStyle, Tokens, Tone};
use detent::{role, Animated};

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
    /// Each part of the handheld looked for, the keys checked by typing.
    Hardware,
    /// How the keyboard works: Shift, Fn and Sym, the arrows, a little practice.
    Lesson,
    Team,
    /// The handheld's name, and its time zone.
    Place,
    Wifi,
    Keys,
    Choices,
    /// The screen going off, the battery saver, Wi-Fi in standby, lift to wake.
    Power,
    Look,
    /// What a computer sent, arriving a row at a time as it is put in place.
    Receiving,
    Finishing,
}

/// The receiving page: when its first row arrives, how far apart they come,
/// how many show at once, and how long the full list stays before "all set".
const RECEIVE_FIRST: f32 = 0.45;
const RECEIVE_STEP: f32 = 0.42;
const RECEIVE_ROWS: usize = 4;
const RECEIVE_HOLD: f32 = 1.3;

/// Setup's pages after the welcome, in order: Next goes one along, Esc one
/// back, and the progress marks count them. A page added here is in the
/// flow everywhere; it needs only its rows, its keys and its drawing.
///
/// Wi-Fi comes first: the clock sets itself from the network, and the rest
/// (the time zone, the keys, the event schedule) is easier with one.
const FLOW: [Page; 9] = [Page::Wifi, Page::Hardware, Page::Lesson, Page::Team, Page::Place, Page::Keys, Page::Choices, Page::Power, Page::Look];

const SURFACE: Color = Color::hex(0x1c1d23);
const SURFACE_FOCUSED: Color = Color::hex(0x272830);
const TRACK: Color = Color::hex(0x2b2d35);
const RING: Color = Color::hex(0xe9e9ee);
/// A keyboard light, lit.
const LIT: Color = Color::hex(0xffc46b);

const LEFT: f32 = 16.0;
const TITLE_CY: f32 = 34.0;
const WORDS_CY: f32 = 51.0;
const ROWS_TOP: f32 = 64.0;
const BAR_CY: f32 = 157.0;

/// A page with an object beside it keeps its words and rows to the left of
/// it, this wide; the object stands on its own spot to the right.
const COLUMN: f32 = 200.0;
const SPOT_X: f32 = 262.0;
const SPOT_Y: f32 = 70.0;
/// The last page's object, in the middle.
const MIDDLE_X: f32 = W * 0.5;
const MIDDLE_Y: f32 = 60.0;
/// How far apart the pages' objects stand along the way through setup: one
/// page on, the last page's object has gone this far to the left (and come
/// nearer, and turned, as a camera moving past it would see it).
const SPACING: f32 = 150.0;
/// After Wi-Fi joins, how long its page stays (its object turning round)
/// before setup carries on by itself.
const JOINED_HOLD: f32 = 1.1;

/// The line under each page's title. Each fits the width it has at its
/// size (a test measures them): nothing on this screen is cut short.
const WORDS_COMPUTER: &str = "On a computer on the same network, open:";
const WORDS_NO_NETWORK: &str = "First it needs a network.";
const WORDS_OR_CABLE: &str = "Or plug in an Ethernet cable.";
const WORDS_OPENS_ITSELF: &str = "Then this page opens itself.";
const WORDS_TEAM: &str = "Its robot's address follows from it.";
const WORDS_PLACE: &str = "Its name, and the time it keeps.";
const WORDS_WIFI: &str = "For the clock and updates.";
const WORDS_KEYS: &str = "For the schedule and assistant.";
const WORDS_TYPING: &str = "Enter keeps it; esc leaves it.";
const WORDS_LOOK: &str = "Change any of these later in Settings.";
const WORDS_DEFAULTS: &str = "Fine as they are: enter goes on.";
const WORDS_HARDWARE: &str = "Each part, looked for. Enter on Keys to type a line.";
/// The welcome's three ways on: the row, and the line under "Hi." while it
/// is focused.
const WAYS: [(&str, &str); 3] = [
    ("Set up here", "On the keys, a page at a time."),
    ("From a computer", "Type it all on a real keyboard."),
    ("Set up later", "To Home; all of it is in Settings."),
];
/// The power page's rows: a name, and the line under the title while it is focused.
const POWER: [(&str, &str); 4] = [
    ("Screen off", "No key this long: it goes dark."),
    ("Saver at 20%", "Fewer frames, a slower processor."),
    ("Wi-Fi sleeps", "Off ten minutes after the screen."),
    ("Lift to wake", "Pick it up and it wakes."),
];
/// The hardware page's chips: three across.
const CHIPS_ACROSS: usize = 3;
const CHOICES: [(&str, &str); 4] = [
    ("Open Robot on link", "Robot opens when one links."),
    ("Match report", "A summary once its log is in."),
    ("SSH access", "A computer may sign in by key."),
    ("Calm version", "Less motion, no transparency."),
];

/// The globe's turn for UTC itself, read off its renders (its land facing
/// front for the Americas at a turn of 0, six hours west); a zone east of it
/// turns the globe on by 15° an hour. Its continents are stylised, so this
/// is a gesture toward the zone rather than a map of it.
const GLOBE_FACING: f32 = -90.0;

/// Hours east of UTC, from an offset as it is written: "UTC−5", "UTC+5:30", "UTC".
fn offset_hours(label: &str) -> f32 {
    let rest = label.trim_start_matches("UTC");
    let (sign, rest) = if let Some(r) = rest.strip_prefix('−').or_else(|| rest.strip_prefix('-')) {
        (-1.0, r)
    } else if let Some(r) = rest.strip_prefix('+') {
        (1.0, r)
    } else {
        return 0.0;
    };
    let (h, m) = rest.split_once(':').unwrap_or((rest, "0"));
    sign * (h.parse::<f32>().unwrap_or(0.0) + m.parse::<f32>().unwrap_or(0.0) / 60.0)
}

/// How long "just a moment" stays, then "all set", before Home.
const WORKING: f32 = 1.5;
const ALL_SET: f32 = 2.6;

// ------------------------------------------------------------------ lesson ---

/// The keyboard lesson's cards, in order.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Card {
    /// Shift, Fn and Sym: tapped, tapped twice, held.
    Modifiers,
    /// F X Z C as the arrows, and a ring to move with them.
    Moving,
    /// A capital with Shift, a symbol with Sym.
    Capitals,
    /// In a text field the letters type; with Fn they are arrows.
    TextArrows,
    /// What else Fn gives.
    FnKeys,
}

const CARDS: [Card; 5] = [Card::Modifiers, Card::Moving, Card::Capitals, Card::TextArrows, Card::FnKeys];

/// Each card's title and the line under it, in the order of `CARDS`.
const CARD_WORDS: [(&str, &str); 5] = [
    ("Shift, Fn and Sym", "Each works three ways; its light says which."),
    ("Moving around", "F X Z C are the arrows, outside typing."),
    ("Capitals and symbols", "Tap Shift, then a letter. Sym types what keys show."),
    ("Arrows while typing", "Here F X Z C type. With Fn they move the cursor."),
    ("More with Fn", "Fn's other keys. Try any of them."),
];

/// The three ways Shift, Fn and Sym work, as the keyboard's driver has
/// them: what to do, what it does, and how the key's light shows it.
const MODIFIER_WAYS: [(&str, &str, Blink); 3] = [
    ("Tap", "for the next key only", Blink::Slow),
    ("Tap twice", "locked on, until tapped again", Blink::Fast),
    ("Hold", "while it is held down", Blink::Steady),
];

/// How a keyboard light shows a modifier: a slow blink, a quick one, or on.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Blink {
    Slow,
    Fast,
    Steady,
}

/// The Fn layer's other keys worth knowing: the keys, what they are, and
/// the key that arrives when they are pressed (none: the shell takes it).
const FN_KEYS: [(&str, &str, Option<Key>); 6] = [
    ("fn bksp", "delete", Some(Key::Delete)),
    ("fn k · n", "home, end", Some(Key::Home)),
    ("fn l · m", "pages", Some(Key::PageUp)),
    ("fn 1 – 0", "F1 – F10", Some(Key::F(1))),
    ("fn u · i", "brightness", None),
    ("sym", "! @ # and more", None),
];

/// The practice on the capitals card and the typing-arrows card.
const TASK_CAPITAL: &str = "a capital, with shift";
const TASK_SYMBOL: &str = "a symbol, with sym";
const TASK_CURSOR: &str = "the cursor moved, with fn and z or c";

/// What the practice field holds at most.
const PRACTICE_MOST: usize = 18;

/// The keyboard lesson: which card it is on, and how the practice has gone.
#[derive(Default)]
struct Lesson {
    card: usize,
    /// Which arrows have arrived on the moving card: up, down, left, right.
    moved: [bool; 4],
    /// The ring on the moving card's little grid, three across and two down.
    at: (usize, usize),
    /// When each of F X Z C last arrived, for its cap to light up.
    pressed: [f32; 4],
    capital: bool,
    symbol: bool,
    /// The practice field, and its cursor (in characters).
    text: String,
    cursor: usize,
    steered: bool,
    /// The Fn keys tried on the last card.
    tried: [bool; 6],
}

impl Lesson {
    fn card(&self) -> Card {
        CARDS[self.card]
    }

    /// Whether the card's practice is a text field (the letters type).
    fn typing(&self) -> bool {
        matches!(self.card(), Card::Capitals | Card::TextArrows)
    }

    /// The card's practice done.
    fn done(&self) -> bool {
        match self.card() {
            Card::Modifiers | Card::FnKeys => true,
            Card::Moving => self.moved.iter().all(|m| *m),
            Card::Capitals => self.capital && self.symbol,
            Card::TextArrows => self.steered,
        }
    }

    /// Arrive at card `i`: its practice field set for it.
    fn arrive(&mut self, i: usize) {
        self.card = i;
        match self.card() {
            Card::Capitals => {
                self.text.clear();
                self.cursor = 0;
            }
            Card::TextArrows => {
                self.text = "catalyst".into();
                self.cursor = self.text.chars().count();
            }
            _ => {}
        }
    }

    fn insert(&mut self, c: char) {
        if self.text.chars().count() >= PRACTICE_MOST {
            return;
        }
        let at = self.text.char_indices().nth(self.cursor).map_or(self.text.len(), |(b, _)| b);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    fn delete_back(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if let Some((b, _)) = self.text.char_indices().nth(self.cursor - 1) {
            self.text.remove(b);
            self.cursor -= 1;
        }
    }

    /// A key on the card's practice; true if it was the card's to take.
    fn practise(&mut self, key: Key, now: f32) -> bool {
        match self.card() {
            Card::Moving => {
                let (dir, (x, y)) = match key {
                    Key::Up => (0, (self.at.0, 0)),
                    Key::Down => (1, (self.at.0, 1)),
                    Key::Left => (2, (self.at.0.saturating_sub(1), self.at.1)),
                    Key::Right => (3, ((self.at.0 + 1).min(2), self.at.1)),
                    _ => return false,
                };
                self.moved[dir] = true;
                self.pressed[dir] = now;
                self.at = (x, y);
                true
            }
            Card::Capitals | Card::TextArrows => {
                match key {
                    Key::Char(c) if !c.is_control() => {
                        self.insert(c);
                        self.capital |= c.is_uppercase();
                        self.symbol |= c.is_ascii_punctuation();
                    }
                    Key::Backspace => self.delete_back(),
                    Key::Delete if self.cursor < self.text.chars().count() => {
                        self.cursor += 1;
                        self.delete_back();
                    }
                    Key::Left | Key::Right | Key::Home | Key::End => {
                        let len = self.text.chars().count();
                        self.cursor = match key {
                            Key::Left => self.cursor.saturating_sub(1),
                            Key::Right => (self.cursor + 1).min(len),
                            Key::Home => 0,
                            _ => len,
                        };
                        self.steered |= self.card() == Card::TextArrows;
                    }
                    _ => return false,
                }
                true
            }
            Card::FnKeys => {
                let found = FN_KEYS.iter().position(|(_, _, k)| match (k, key) {
                    (Some(Key::F(_)), Key::F(_)) => true,
                    (Some(Key::Home), Key::End) | (Some(Key::PageUp), Key::PageDown) => true,
                    (Some(k), key) => *k == key,
                    (None, _) => false,
                });
                match found {
                    Some(i) => {
                        self.tried[i] = true;
                        true
                    }
                    None => false,
                }
            }
            Card::Modifiers => false,
        }
    }
}

// ------------------------------------------------------------------- setup ---

pub struct Setup {
    page: Page,
    focus: usize,
    /// Seconds since the page arrived, for its elements rising in.
    entered: f32,
    /// Seconds since setup opened, for the keyboard lights' blinking.
    clock: f32,
    /// +1 when the page came from the right (forward), −1 from the left.
    from: f32,
    team: String,
    /// The handheld's name as typed on its page.
    name: String,
    keys: Keys,
    /// A key being typed: which service, and what has been typed.
    typing: Option<(usize, String)>,
    lesson: Lesson,
    ground: Vec<u32>,
    wifi: Option<crate::wifi::Wifi>,
    /// The Wi-Fi state's generation when last drawn.
    wifi_seen: u64,
    /// The words under a page's title, when something happened to say.
    note: Option<String>,
    /// The address and one-time code a computer uses, while it is offered.
    pub remote: Option<crate::remote::Remote>,
    /// Opened from Settings rather than on first start: one page alone (the
    /// computer page, or the keyboard lesson), and back to Settings when done.
    from_settings: bool,
    /// The hardware page's parts, looked for once it is first shown.
    hardware: Option<crate::hwcheck::Check>,
    /// The computer page with no network yet: seconds since it last looked
    /// for one, to offer the page as soon as there is.
    looked_for_network: f32,
    /// Wi-Fi was switched on by arriving at its page (so the radio warms up
    /// while the page is read); switched off again if it is passed by
    /// without joining anything.
    wifi_by_setup: bool,
    /// What a computer sent, a row each, for the receiving page; and that
    /// page's list scrolling as it fills.
    received: Vec<(String, String)>,
    received_scroll: detent::Animated,
    /// The pages this setup goes through: all of `FLOW`, or, once a computer
    /// has sent its answers on first start, those only the handheld can do
    /// (`left_for_handheld`).
    flow: Vec<Page>,
    /// A computer sent the answers on first start: setup goes on from what
    /// it received, and never back to the welcome.
    sent_from_computer: bool,
    /// The way through setup as a space: each page's object stands at its
    /// own place along it, and `travel` is where the camera is (in pages),
    /// gliding to the page in front. The object of the page just left is
    /// carried off as the next one comes.
    travel: Animated,
    leaving: Option<(&'static str, f32)>,
    /// A turn of the object in front on top of where it rests: a whole one
    /// when something is done (a network joined, a key kept), a nudge when
    /// something changes (a digit typed, a choice switched).
    spin: Animated,
    /// The object in front changing on the same page (the welcome's ways,
    /// the last page): the old one turns away and the new one turns in.
    flip: Animated,
    flip_from: Option<&'static str>,
    shown: Option<&'static str>,
    /// Wi-Fi joined from its page: when, so setup carries on by itself.
    joined: Option<f32>,
    /// Where the object in front rests, turning there when it changes (the
    /// globe to a time zone chosen).
    look: Animated,
    calm: bool,
}

/// What is left to do on the handheld once a computer has sent its answers
/// on first start, in `FLOW`'s order. The page on the computer has nothing
/// for the hardware check, the keyboard lesson, the name and time zone or
/// the power choices, so those always follow. Wi-Fi is passed by (there is a
/// network: the page came over it; any networks sent are joined), and so are
/// the keys (typed on the computer, or left for Settings). The team, the
/// choices and the look are asked again only if the computer left them all
/// unanswered.
fn left_for_handheld(a: &crate::remote::Answers) -> Vec<Page> {
    let choices_sent = [a.open_robot, a.match_report, a.ssh, a.calm].iter().any(Option::is_some);
    let look_sent = a.accent.is_some() || a.light.is_some() || a.text.is_some() || a.home.is_some();
    FLOW.iter()
        .copied()
        .filter(|page| match page {
            Page::Wifi | Page::Keys => false,
            Page::Team => a.team.is_none(),
            Page::Choices => !choices_sent,
            Page::Look => !look_sent,
            _ => true,
        })
        .collect()
}

impl Setup {
    pub fn new() -> Self {
        Setup {
            page: Page::Welcome,
            focus: 0,
            entered: 0.0,
            clock: 0.0,
            from: 1.0,
            team: String::new(),
            name: String::new(),
            keys: Keys::load(),
            typing: None,
            lesson: Lesson::default(),
            ground: brand::ground(),
            wifi: None,
            wifi_seen: u64::MAX,
            note: None,
            remote: None,
            from_settings: false,
            hardware: None,
            looked_for_network: 0.0,
            wifi_by_setup: false,
            received: Vec::new(),
            received_scroll: detent::Animated::new(0.0, role::release()).with_rest(0.01),
            flow: FLOW.to_vec(),
            sent_from_computer: false,
            travel: Animated::new(0.0, crate::gfx::spatial::slow()).with_rest(0.002),
            leaving: None,
            spin: Animated::new(0.0, role::release()).with_rest(0.2),
            flip: Animated::new(1.0, crate::gfx::spatial::default()).with_rest(0.004),
            flip_from: None,
            shown: Some("handheld"),
            joined: None,
            look: Animated::new(0.0, crate::gfx::spatial::slow()).with_rest(0.05),
            calm: false,
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

    /// The receiving page with what a computer sent, for the gallery.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing_received(answers: &crate::remote::Answers) -> Self {
        let mut setup = Setup { from_settings: true, ..Self::new() };
        setup.received = crate::remote::received(answers);
        setup.go(Page::Receiving, true);
        setup
    }

    /// First start after a computer sent `answers`: the first of the pages
    /// left for the handheld, for the gallery.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn after_computer(answers: &crate::remote::Answers) -> Self {
        let mut setup = Setup { flow: left_for_handheld(answers), sent_from_computer: true, ..Self::new() };
        setup.go(setup.flow[0], true);
        setup
    }

    /// The computer page as it is with no network yet, for the gallery.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn without_network() -> Self {
        let mut setup = Setup { from_settings: true, ..Self::new() };
        setup.go(Page::Computer, true);
        setup
    }

    /// Settings › Help › Keyboard lesson: the lesson on its own.
    pub fn lesson() -> Self {
        let mut setup = Setup { from_settings: true, ..Self::new() };
        setup.go(Page::Lesson, true);
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

    /// What a computer sent: shown arriving, put in place, and on first
    /// start followed by the pages only the handheld can do.
    fn take_answers(&mut self, answers: crate::remote::Answers, cx: &mut Cx) {
        // What arrived, to show it coming in, row by row; then in place.
        self.received = crate::remote::received(&answers);
        // On first start, what only the handheld can do follows (the
        // keyboard lesson above all); from Settings, this was the whole job.
        if !self.from_settings {
            self.flow = left_for_handheld(&answers);
            self.sent_from_computer = true;
        }
        self.apply(answers, cx);
        if !self.from_settings {
            // What the computer sent is in place whatever happens next.
            crate::mark_setup_done();
        }
        self.received_scroll.jump(0.0);
        self.go(Page::Receiving, true);
    }

    fn go(&mut self, page: Page, forward: bool) {
        let (before, from_slot) = (self.hero(), self.slot());
        self.page = page;
        self.focus = 0;
        self.entered = 0.0;
        self.from = if forward { 1.0 } else { -1.0 };
        self.typing = None;
        self.note = None;
        self.joined = None;
        // The radio takes seconds to start: on as the page arrives, so the
        // networks are listed by the time "Choose a network" is pressed.
        if page == Page::Wifi && !crate::wifi::switched_on() {
            crate::wifi::switch(true);
            self.wifi_by_setup = true;
        }
        self.wifi = (page == Page::Wifi).then(crate::wifi::Wifi::open);
        if page != Page::Computer {
            self.remote = None;
        }
        match page {
            Page::Lesson => self.lesson.arrive(if forward { 0 } else { CARDS.len() - 1 }),
            Page::Place => self.name = crate::device::suggested_name(&self.team),
            Page::Hardware if self.hardware.is_none() => self.hardware = Some(crate::hwcheck::Check::run()),
            _ => {}
        }
        // Pages that offer what is already chosen (fine as it is) start on
        // Next, so Enter goes on and setup goes quickly; Up goes into them.
        // So does Wi-Fi once there is a network.
        let online = || crate::sys::address("wlan0").is_some_and(|a| a.contains('.'));
        if matches!(page, Page::Hardware | Page::Keys | Page::Choices | Page::Power | Page::Look) || (page == Page::Wifi && online()) {
            self.focus = self.rows();
        }
        // The camera moves on to this page's place, carrying the last page's
        // object off as this one's comes in. Calm, it is simply there.
        let to = self.slot();
        if self.calm {
            self.travel.jump(to);
            self.leaving = None;
        } else if to != from_slot {
            self.leaving = before.map(|o| (o, from_slot));
            self.travel.set(to);
        }
        self.spin.jump(0.0);
        self.flip.jump(1.0);
        self.flip_from = None;
        self.shown = self.hero();
        self.look.jump(self.rest_yaw());
    }

    /// Where a page stands along the way through setup, in pages.
    fn slot(&self) -> f32 {
        match self.page {
            Page::Welcome => 0.0,
            Page::Computer | Page::Receiving => 1.0,
            Page::Finishing => self.flow.len() as f32 + 1.0,
            page => self.flow.iter().position(|p| *p == page).map_or(1.0, |i| i as f32 + 1.0),
        }
    }

    /// The object a page shows, if it has room for one beside its words:
    /// the handheld to welcome (the laptop while "From a computer" is
    /// focused), the network, the team's robot, the globe for the time
    /// zone, a key, a switch, the battery; the handheld at work on the last
    /// page, then a trophy. The pages laid out across the whole panel (the
    /// hardware's parts, the lesson's cards, the look and its picture of
    /// Home, the code for a computer) have none.
    fn hero(&self) -> Option<&'static str> {
        Some(match self.page {
            Page::Welcome if self.focus == 1 => "laptop",
            Page::Welcome => "handheld",
            Page::Computer if self.remote.is_none() => "laptop",
            Page::Wifi => "wifi",
            Page::Team => "robot",
            Page::Place => "globe",
            Page::Keys => "key",
            Page::Choices => "toggle",
            Page::Power => "battery",
            Page::Finishing if self.entered >= WORKING => "trophy",
            Page::Finishing => "handheld",
            _ => return None,
        })
    }

    /// Something done: the object in front turns once round.
    fn celebrate(&mut self) {
        if !self.calm {
            self.spin.jump(self.spin.value() - 360.0);
            self.spin.set(0.0);
        }
    }

    /// Something changed: the object in front gives a little turn that way.
    fn nudge(&mut self, way: f32) {
        if !self.calm {
            self.spin.nudge(way * 220.0);
        }
    }

    /// Where the object in front rests, turned: the globe by the time zone
    /// chosen (its offset from UTC, an hour to 15°, see `GLOBE_FACING`), so
    /// choosing one turns it; the handheld at work turning.
    fn rest_yaw(&self) -> f32 {
        match self.page {
            Page::Place => {
                let offset = crate::device::zone_offset(&crate::device::zone()).unwrap_or_default();
                GLOBE_FACING - 15.0 * offset_hours(&offset)
            }
            Page::Finishing if self.entered < WORKING && !self.calm => self.entered * 70.0,
            _ => 0.0,
        }
    }

    /// The power page's rows: lift to wake only where there is a motion sensor.
    fn power_rows(&self) -> usize {
        if crate::imu::has_motion() {
            POWER.len()
        } else {
            POWER.len() - 1
        }
    }

    fn change_power(&mut self, row: usize, step: i32, cx: &mut Cx) {
        let s = &mut cx.settings;
        match row {
            0 => {
                let all = crate::standby::SCREEN_OFF_CHOICES;
                let at = all.iter().position(|v| *v == s.screen_off).unwrap_or(1) as i32;
                s.screen_off = all[(at + step).rem_euclid(all.len() as i32) as usize];
            }
            1 => s.battery.saver_auto = !s.battery.saver_auto,
            2 => s.battery.wifi_sleeps = !s.battery.wifi_sleeps,
            _ => s.lift_wake = !s.lift_wake,
        }
    }

    /// The hardware page: the arrows go round the chips (three across) and
    /// on to Next; Enter on Keys opens the line to type.
    fn hardware_key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let n = self.rows();
        match key {
            Key::Back => return self.back(),
            Key::Tab => return self.next(cx),
            Key::Enter if self.focus >= n => return self.next(cx),
            Key::Enter => return self.hardware.as_ref().and_then(|h| h.open(self.focus)).map_or(Nav::Stay, Nav::Push),
            Key::Right => self.focus = (self.focus + 1).min(n),
            Key::Left => self.focus = self.focus.saturating_sub(1),
            Key::Down => self.focus = if self.focus + CHIPS_ACROSS < n { self.focus + CHIPS_ACROSS } else { n },
            Key::Up if self.focus >= n => self.focus = n.saturating_sub(1),
            Key::Up => self.focus = self.focus.saturating_sub(CHIPS_ACROSS),
            _ => {}
        }
        Nav::Stay
    }

    /// Keep what the page being left holds: the team, the keys, the name.
    fn keep(&mut self, cx: &mut Cx) {
        match self.page {
            // Passed by without joining a network: the radio setup switched
            // on goes off again (it is the largest draw after the screen).
            Page::Wifi if self.wifi_by_setup => {
                if !crate::sys::address("wlan0").is_some_and(|a| a.contains('.')) {
                    crate::wifi::switch(false);
                }
                self.wifi_by_setup = false;
            }
            Page::Team => self.save_team(cx),
            Page::Keys => {
                self.keys.save().ok();
            }
            // A name cleared away leaves the one there was.
            Page::Place => {
                let name = crate::device::clean_name(&self.name);
                if !name.is_empty() && name != crate::device::name() {
                    if let Err(why) = crate::device::set_name(&name) {
                        *cx.say = Some(why);
                    }
                }
            }
            _ => {}
        }
    }

    fn next(&mut self, cx: &mut Cx) -> Nav {
        self.keep(cx);
        // The lesson on its own: done, back to Settings.
        if self.from_settings && self.page == Page::Lesson {
            *cx.say = Some("that's the keyboard".into());
            return Nav::Pop;
        }
        let page = match self.page {
            Page::Welcome | Page::Computer => FLOW[0],
            Page::Finishing => Page::Finishing,
            page => match self.flow.iter().position(|p| *p == page) {
                Some(i) if i + 1 < self.flow.len() => self.flow[i + 1],
                // The last page: everything chosen is kept, and setup is not shown again.
                _ => {
                    crate::save_settings(cx.settings);
                    crate::mark_setup_done();
                    Page::Finishing
                }
            },
        };
        self.go(page, true);
        Nav::Stay
    }

    fn back(&mut self) -> Nav {
        if self.from_settings {
            return if self.page == Page::Lesson { Nav::Pop } else { Nav::Stay };
        }
        let page = match self.page {
            Page::Welcome | Page::Computer => Page::Welcome,
            Page::Finishing => return Nav::Stay,
            page => match self.flow.iter().position(|p| *p == page) {
                Some(i) if i > 0 => self.flow[i - 1],
                // What a computer sent is in place: nothing to go back to.
                _ if self.sent_from_computer => return Nav::Stay,
                _ => Page::Welcome,
            },
        };
        if page != self.page {
            self.go(page, false);
        }
        Nav::Stay
    }

    fn save_team(&mut self, cx: &mut Cx) {
        let team = self.team.trim().to_owned();
        if team.is_empty() {
            return;
        }
        crate::save_robot_address(&team);
        // In the simulator the demo robot stands in for the team's own: keep it.
        if !cx.robot.is_demo() {
            *cx.robot = Robot::connect(&crate::screens::resolve_address(&team));
        }
    }

    /// How many rows the page has, not counting its Next button.
    fn rows(&self) -> usize {
        match self.page {
            Page::Welcome => 3,
            Page::Computer | Page::Lesson | Page::Receiving | Page::Finishing => 0,
            Page::Team => 1,
            Page::Place => 2,
            Page::Wifi => 2,
            Page::Keys => crate::keys::SETUP_SERVICES,
            Page::Choices => 4,
            Page::Hardware => self.hardware.as_ref().map_or(0, |h| h.states.len()),
            Page::Power => self.power_rows(),
            Page::Look => 4,
        }
    }

    /// Whether the page has a Next button below its rows.
    fn has_next(&self) -> bool {
        matches!(self.page, Page::Hardware | Page::Team | Page::Place | Page::Wifi | Page::Keys | Page::Choices | Page::Power | Page::Look)
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

    /// The lesson's keys: Enter on, Esc back, Tab past the lesson; the rest
    /// are the card's practice.
    fn lesson_key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Tab => return self.next(cx),
            Key::Enter => {
                if self.lesson.card + 1 < CARDS.len() {
                    self.lesson.arrive(self.lesson.card + 1);
                    (self.entered, self.from) = (0.0, 1.0);
                } else {
                    return self.next(cx);
                }
            }
            Key::Back => {
                if self.lesson.card > 0 {
                    self.lesson.arrive(self.lesson.card - 1);
                    (self.entered, self.from) = (0.0, -1.0);
                } else {
                    return self.back();
                }
            }
            key => {
                self.lesson.practise(key, self.clock);
            }
        }
        Nav::Stay
    }

    // ---- drawing ----

    fn header(&self, g: &mut Gfx) {
        brand::mark(g, LEFT + 4.0, 13.0, 12.0, CRIMSON, 1.0);
        // The steps: those done, the one under way in crimson, those to come.
        // (Not for a page opened alone from Settings.)
        let at = self.flow.iter().position(|p| *p == self.page);
        if self.from_settings || (at.is_none() && self.page != Page::Finishing) {
            return;
        }
        // Beside the mark, at the left: the right is the objects' place.
        let (w, gap) = (12.0, 4.0);
        let x0 = LEFT + 16.0;
        for i in 0..self.flow.len() {
            let color = match at {
                Some(at) if i == at => CRIMSON,
                Some(at) if i < at => INK_THIN,
                None => INK_THIN,
                _ => TRACK,
            };
            g.canvas.rrect(Rect::new(x0 + i as f32 * (w + gap), 12.0, w, 3.0), 1.5, color, 1.0);
        }
    }

    /// How wide the page's words and rows may be: to the left of its object, or across.
    fn column(&self) -> f32 {
        if self.hero().is_some() && self.page != Page::Finishing {
            COLUMN
        } else {
            W - 2.0 * LEFT
        }
    }

    fn heading(&self, g: &mut Gfx, title: &str, words: &str) {
        let width = self.column();
        let (a, dx, dy) = self.reveal(0);
        let style = TextStyle::sans(18.0, 600.0);
        let title = g.text.fit(title, &style, width);
        g.text_alpha(LEFT + dx, TITLE_CY + dy, &title, style, INK_STRONG, a);
        let (a, dx, dy) = self.reveal(1);
        let words = self.note.as_deref().unwrap_or(words);
        let fitted = g.text.fit(words, &TextStyle::sans(11.0, 400.0), width);
        g.text_alpha(LEFT + dx, WORDS_CY + dy, &fitted, TextStyle::sans(11.0, 400.0), if self.note.is_some() { RING } else { INK_THIN }, a);
    }

    /// The page's objects: the one in front on its spot, and the last page's
    /// on its way off. Each stands where the camera now sees it: a page
    /// ahead is further off to the right (smaller, turned away), a page
    /// passed has come nearer and gone left; each fades as it leaves the
    /// middle of the way. A pool of light in the accent lies under each.
    fn draw_objects(&mut self, g: &mut Gfx) {
        let travel = self.travel.value();
        let middle = self.page == Page::Finishing;
        let spot = if middle { (MIDDLE_X, MIDDLE_Y) } else { (SPOT_X, SPOT_Y) };
        if let Some((object, slot)) = self.leaving {
            let d = slot - travel;
            if d.abs() < 0.999 && !g.calm {
                stand(g, object, (SPOT_X, SPOT_Y), d, 0.0, 1.0);
            } else {
                self.leaving = None;
            }
        }
        let Some(object) = self.hero() else { return };
        let d = self.slot() - travel;
        let yaw = self.look.value() + self.spin.value();
        // Changing on its page: the old one turns away edge on, the new one
        // turns in from the other side and settles; a little smaller in
        // between, as a card flipped.
        let f = self.flip.value();
        let dip = 1.0 - 0.08 * (std::f32::consts::PI * f.clamp(0.0, 1.0)).sin();
        match self.flip_from {
            Some(old) if f < 0.5 && !g.calm => stand(g, old, spot, d, yaw + f * 180.0, dip),
            _ => stand(g, object, spot, d, yaw - (1.0 - f) * 180.0, dip),
        }
    }

    /// A row: a label at the left and a value at the right, `h` tall.
    #[allow(clippy::too_many_arguments)]
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
            // Beside its name, cut short rather than run into it (a long
            // time zone in the column beside an object).
            let style = TextStyle::code(9.5);
            let room = r.w - 33.0 - g.measure(label, label_style);
            let value = g.text.fit(value, &style, room.max(24.0));
            let vw = g.measure(&value, style);
            g.text_alpha(r.right() - 11.0 - vw, r.cy(), &value, style, if on { INK_STRONG } else { INK_THIN }, a);
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
        let label = if self.flow.last() == Some(&self.page) { "Finish" } else { "Next" };
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

    /// A key as it is printed on the keyboard: a small cap with its legend,
    /// lit when `lit` (0 to 1) says it was just pressed.
    fn keycap(&self, g: &mut Gfx, r: Rect, legend: &str, lit: f32, a: f32) {
        g.canvas.rrect(r, 4.0, SURFACE_FOCUSED, a);
        if lit > 0.0 {
            g.canvas.rrect(r, 4.0, RING, a * lit * 0.22);
        }
        g.canvas.rrect(Rect::new(r.x + 2.0, r.bottom() - 1.0, r.w - 4.0, 1.0), 0.5, Color::BLACK, a * 0.35);
        let style = TextStyle::code(9.5);
        let lw = g.measure(legend, style);
        g.text_alpha(r.cx() - lw * 0.5, r.cy() - 0.5, legend, style, INK_STRONG, a);
    }

    /// A practice task: done, or still to do.
    fn task(&self, g: &mut Gfx, x: f32, cy: f32, label: &str, done: bool, a: f32) -> f32 {
        let style = TextStyle::sans(10.5, if done { 600.0 } else { 500.0 });
        let w = g.measure(label, style) + 30.0;
        let r = Rect::new(x, cy - 9.0, w, 18.0);
        g.canvas.rrect(r, 9.0, if done { SURFACE_FOCUSED } else { SURFACE }, a);
        if done {
            // A tick, drawn: in a filled disc, as a check box ticked.
            let x = r.x + 10.0;
            g.canvas.circle(x, cy, 5.5, INK_STRONG, a);
            g.canvas.polyline(&[(x - 2.6, cy + 0.2), (x - 0.8, cy + 2.1), (x + 2.8, cy - 2.0)], 1.5, Color::hex(0x101116), a);
        } else {
            g.canvas.stroke_rrect(Rect::new(r.x + 5.0, cy - 5.0, 10.0, 10.0), 5.0, 1.2, INK_FAINT, a);
        }
        g.text_alpha(r.x + 20.0, cy, label, style, if done { INK_STRONG } else { INK_THIN }, a);
        w
    }

    /// The practice field: what has been typed, and the cursor in it.
    fn practice_field(&self, g: &mut Gfx, y: f32, placeholder: &str) {
        let (a, dx, dy) = self.reveal(2);
        let field = Rect::new(LEFT + dx, y + dy, W - 2.0 * LEFT, 28.0);
        g.canvas.rrect(field, 10.0, SURFACE_FOCUSED, a);
        g.canvas.stroke_rrect(field.inset(-2.0), 12.0, 1.5, RING, 0.9 * a);
        let style = TextStyle::sans(14.0, 500.0);
        let l = &self.lesson;
        if l.text.is_empty() {
            g.text_alpha(field.x + 12.0, field.cy(), placeholder, style, INK_FAINT, a);
        } else {
            g.text_alpha(field.x + 12.0, field.cy(), &l.text, style, INK_STRONG, a);
        }
        let before: String = l.text.chars().take(l.cursor).collect();
        let cx = field.x + 12.0 + if before.is_empty() { 0.0 } else { g.measure(&before, style) };
        g.canvas.rrect(Rect::new(cx, field.cy() - 8.0, 1.5, 16.0), 0.75, CRIMSON, a);
    }

    /// A keyboard light as it shows a modifier, at this moment.
    fn light(&self, g: &mut Gfx, cx: f32, cy: f32, blink: Blink, calm: bool, a: f32) {
        let on = match blink {
            // Calm: no blinking; slow and quick are shown half and three-quarters lit.
            _ if calm => match blink {
                Blink::Slow => 0.45,
                Blink::Fast => 0.75,
                Blink::Steady => 1.0,
            },
            Blink::Slow => ((self.clock * std::f32::consts::TAU * 0.8).sin() * 0.5 + 0.5).powf(0.6),
            Blink::Fast => {
                if (self.clock * 6.0).fract() < 0.5 {
                    1.0
                } else {
                    0.15
                }
            }
            Blink::Steady => 1.0,
        };
        g.canvas.circle(cx, cy, 4.5, TRACK, a);
        g.canvas.circle(cx, cy, 4.5, LIT, a * on);
        if on > 0.5 && !calm {
            g.canvas.glow(cx, cy, 10.0, LIT, 0.25 * a * on);
        }
    }

    fn draw_lesson(&self, g: &mut Gfx) {
        let l = &self.lesson;
        let calm = g.calm;
        let (title, words) = CARD_WORDS[l.card];
        self.heading(g, title, words);
        match l.card() {
            Card::Modifiers => {
                for (i, (how, what, blink)) in MODIFIER_WAYS.iter().enumerate() {
                    let (a, dx, dy) = self.reveal(2 + i);
                    let y = ROWS_TOP + 8.0 + i as f32 * 24.0 + dy;
                    let r = Rect::new(LEFT + dx, y, W - 2.0 * LEFT, 20.0);
                    g.canvas.rrect(r, 10.0, SURFACE, a);
                    self.light(g, r.x + 13.0, r.cy(), *blink, calm, a);
                    g.text_alpha(r.x + 26.0, r.cy(), how, TextStyle::sans(11.5, 600.0), INK_STRONG, a);
                    g.text_alpha(r.x + 92.0, r.cy(), what, TextStyle::sans(10.5, 400.0), INK_THIN, a);
                }
            }
            Card::Moving => {
                let (a, dx, dy) = self.reveal(2);
                // The four keys as they sit: F above X, Z and C either side.
                let (cw, ch, gap) = (30.0, 22.0, 3.0);
                let (x0, y0) = (LEFT + 4.0 + dx, ROWS_TOP + 6.0 + dy);
                let caps = [("F ↑", x0 + cw + gap, y0), ("X ↓", x0 + cw + gap, y0 + ch + gap), ("Z ←", x0, y0 + ch + gap), ("C →", x0 + 2.0 * (cw + gap), y0 + ch + gap)];
                for (dir, (legend, x, y)) in caps.iter().enumerate() {
                    let since = self.clock - l.pressed[dir];
                    let lit = if l.moved[dir] && since < 0.35 { 1.0 - since / 0.35 } else { 0.0 };
                    self.keycap(g, Rect::new(*x, *y, cw, ch), legend, lit, a);
                }
                // A little grid of tiles, and the ring the keys move.
                let (a, dx, dy) = self.reveal(3);
                let (tw, th, tg) = (40.0, 22.0, 5.0);
                let gx = W - LEFT - 3.0 * tw - 2.0 * tg + dx;
                for row in 0..2 {
                    for col in 0..3 {
                        let r = Rect::new(gx + col as f32 * (tw + tg), ROWS_TOP + 6.0 + row as f32 * (th + tg) + dy, tw, th);
                        let here = (col, row) == l.at;
                        g.canvas.rrect(r, 6.0, if here { SURFACE_FOCUSED } else { SURFACE }, a);
                        if here {
                            g.canvas.stroke_rrect(r.inset(-2.0), 8.0, 1.5, RING, 0.9 * a);
                        }
                    }
                }
                let (a, ..) = self.reveal(4);
                let mut x = LEFT;
                for (i, name) in ["up", "down", "left", "right"].iter().enumerate() {
                    x += self.task(g, x, 134.0, name, l.moved[i], a) + 5.0;
                }
            }
            Card::Capitals => {
                self.practice_field(g, ROWS_TOP + 6.0, "type here");
                let (a, ..) = self.reveal(3);
                let w = self.task(g, LEFT, 116.0, TASK_CAPITAL, l.capital, a);
                self.task(g, LEFT + w + 6.0, 116.0, TASK_SYMBOL, l.symbol, a);
            }
            Card::TextArrows => {
                self.practice_field(g, ROWS_TOP + 6.0, "");
                let (a, ..) = self.reveal(3);
                self.task(g, LEFT, 116.0, TASK_CURSOR, l.steered, a);
            }
            Card::FnKeys => {
                let col_w = (W - 2.0 * LEFT - 6.0) * 0.5;
                for (i, (keys, what, _)) in FN_KEYS.iter().enumerate() {
                    let (a, dx, dy) = self.reveal(2 + i / 2);
                    let x = LEFT + (i % 2) as f32 * (col_w + 6.0) + dx;
                    let y = ROWS_TOP + 4.0 + (i / 2) as f32 * 25.0 + dy;
                    let r = Rect::new(x, y, col_w, 21.0);
                    g.canvas.rrect(r, 10.5, if l.tried[i] { SURFACE_FOCUSED } else { SURFACE }, a);
                    let style = TextStyle::code(9.0);
                    let kw = g.measure(keys, style) + 12.0;
                    let cap = Rect::new(r.x + 4.0, r.y + 3.0, kw, r.h - 6.0);
                    g.canvas.rrect(cap, 4.0, TRACK, a);
                    g.text_alpha(cap.x + 6.0, cap.cy(), keys, style, INK_STRONG, a);
                    g.text_alpha(cap.right() + 7.0, r.cy(), what, TextStyle::sans(10.5, if l.tried[i] { 600.0 } else { 400.0 }), if l.tried[i] { INK_STRONG } else { INK_THIN }, a);
                }
            }
        }
        // The cards: where the lesson is, beside what the keys do.
        let (a, ..) = self.reveal(7);
        for i in 0..CARDS.len() {
            let x = W - LEFT - (CARDS.len() - 1 - i) as f32 * 10.0 - 3.0;
            let (radius, ink) = if i == l.card { (3.0, CRIMSON) } else { (2.0, if i < l.card { INK_THIN } else { TRACK }) };
            g.canvas.circle(x, BAR_CY, radius, ink, a);
        }
        let last = l.card + 1 == CARDS.len();
        let hint = match (last, self.from_settings) {
            (true, true) => "enter: done · esc: back",
            (true, false) => "enter: on to setup · esc: back",
            (false, _) if l.done() => "enter: next · esc: back · tab: skip",
            (false, _) => "enter: next · tab: skip the lesson",
        };
        g.text_alpha(LEFT, BAR_CY, hint, TextStyle::code(9.0), INK_FAINT, a);
    }

    /// The hardware page: a chip for each part, three across, ticked when
    /// found, marked amber when not; the line under the title says what the
    /// chip in focus found.
    fn draw_hardware(&self, g: &mut Gfx) {
        use crate::hwcheck::State;
        let Some(check) = &self.hardware else { return };
        let words = match check.states.get(self.focus) {
            // The keys, not yet typed: what to do.
            Some((_, State::Needs(_))) => WORDS_HARDWARE.to_owned(),
            Some((part, state)) => format!("{}: {}", part.name(), state.words()),
            None => {
                let (found, all) = check.found();
                format!("{found} of {all} found. It is in Settings › About too.")
            }
        };
        self.heading(g, "Check the hardware", &words);
        let gap = 5.0;
        let cw = (W - 2.0 * LEFT - gap * (CHIPS_ACROSS - 1) as f32) / CHIPS_ACROSS as f32;
        for (i, (part, state)) in check.states.iter().enumerate() {
            let (a, dx, dy) = self.reveal(2 + i / CHIPS_ACROSS);
            let (col, row) = (i % CHIPS_ACROSS, i / CHIPS_ACROSS);
            let r = Rect::new(LEFT + col as f32 * (cw + gap) + dx, ROWS_TOP + 1.0 + row as f32 * 21.0 + dy, cw, 18.0);
            let focused = self.focus == i;
            g.canvas.rrect(r, 9.0, if focused { SURFACE_FOCUSED } else { SURFACE }, a);
            // The mark: a tick in a disc when found, an amber disc with a bar
            // when not, a ring while it waits for a hand (the keys).
            let (mx, my) = (r.x + 10.0, r.cy());
            match state {
                State::Ok(_) => {
                    g.canvas.circle(mx, my, 5.0, INK_STRONG, a);
                    g.canvas.polyline(&[(mx - 2.4, my + 0.2), (mx - 0.7, my + 1.9), (mx + 2.5, my - 1.8)], 1.4, Color::hex(0x101116), a);
                }
                State::Missing(_) => {
                    g.canvas.circle(mx, my, 5.0, LIT, a);
                    g.canvas.rrect(Rect::new(mx - 0.75, my - 3.0, 1.5, 3.8), 0.75, Color::hex(0x101116), a);
                    g.canvas.circle(mx, my + 2.4, 0.85, Color::hex(0x101116), a);
                }
                State::Needs(_) => g.canvas.stroke_rrect(Rect::new(mx - 4.5, my - 4.5, 9.0, 9.0), 4.5, 1.2, INK_FAINT, a),
            }
            let style = TextStyle::sans(10.5, if focused { 600.0 } else { 500.0 });
            let name = g.text.fit(part.short(), &style, r.w - 24.0);
            g.text_alpha(r.x + 19.0, r.cy(), &name, style, if state.ok() { INK_STRONG } else { INK_THIN }, a);
            if focused {
                g.canvas.stroke_rrect(r.inset(-2.0), 11.0, 1.5, RING, 0.9 * a);
            }
        }
        self.bar(g, if self.focus >= self.rows() { "enter: next · arrows: the parts" } else { "arrows move · enter on keys · tab skip" });
    }

    /// How many of what was received have arrived on the page so far.
    fn received_shown(&self) -> usize {
        if self.entered < RECEIVE_FIRST {
            return 0;
        }
        (((self.entered - RECEIVE_FIRST) / RECEIVE_STEP) as usize + 1).min(self.received.len())
    }

    /// Receiving: what the computer sent, a row at a time, each rising in
    /// with a tick as it is put in place; the newest kept in view.
    fn draw_receiving(&self, g: &mut Gfx) {
        let shown = self.received_shown();
        let all = shown == self.received.len();
        self.heading(g, "Receiving", if all { "All in place." } else { "From the computer, putting each in place." });
        let step = 21.0;
        let scroll = self.received_scroll.value();
        let area = Rect::new(0.0, ROWS_TOP, W, RECEIVE_ROWS as f32 * step);
        g.canvas.set_clip(area);
        for (k, (name, value)) in self.received.iter().enumerate().take(shown) {
            // Each row rises in on `release` from just below where it lands.
            let since = self.entered - (RECEIVE_FIRST + k as f32 * RECEIVE_STEP);
            let left = if g.calm { 0.0 } else { role::release().solve(1.0, 0.0, since.max(0.0)).0 };
            let a = ((1.0 - left) * 1.5).clamp(0.0, 1.0);
            let y = ROWS_TOP + 1.0 + (k as f32 - scroll) * step + left * 8.0;
            if y + step < ROWS_TOP || y > area.bottom() {
                continue;
            }
            let r = Rect::new(LEFT, y, W - 2.0 * LEFT, 18.0);
            g.canvas.rrect(r, 9.0, SURFACE, a);
            // The tick, in a disc, as a task done.
            let (mx, my) = (r.x + 10.0, r.cy());
            g.canvas.circle(mx, my, 5.0, INK_STRONG, a);
            g.canvas.polyline(&[(mx - 2.4, my + 0.2), (mx - 0.7, my + 1.9), (mx + 2.5, my - 1.8)], 1.4, Color::hex(0x101116), a);
            let label = TextStyle::sans(11.0, 600.0);
            g.text_alpha(r.x + 21.0, r.cy(), name, label, INK_STRONG, a);
            if !value.is_empty() {
                let style = TextStyle::code(9.5);
                let room = r.w - 32.0 - g.measure(name, label) - 12.0;
                let fitted = g.text.fit(value, &style, room.max(20.0));
                let vw = g.measure(&fitted, style);
                g.text_alpha(r.right() - 11.0 - vw, r.cy(), &fitted, style, INK_THIN, a);
            }
        }
        g.canvas.clear_clip();
        // The subtitle already says "All in place." once everything has
        // landed, so the footer keeps counting rather than repeating it.
        let count = format!("{shown} of {} received", self.received.len());
        g.text_alpha(LEFT, BAR_CY, &count, TextStyle::code(9.0), INK_FAINT, 1.0);
    }

    fn draw_place(&self, g: &mut Gfx) {
        self.heading(g, "Name and time zone", WORDS_PLACE);
        let row_w = self.column();
        let shown = if self.focus == 0 { format!("{}▏", self.name) } else { self.name.clone() };
        self.row(g, 0, ROWS_TOP + 6.0, row_w, 21.0, "Name", &shown, true);
        // The place on the row (it shares the column with the globe); its
        // offset on the line under the rows, with the time.
        let zone = crate::device::zone();
        let city = crate::device::zone_city(&zone);
        let value = format!("{} ›", city.split(" · ").next().unwrap_or(&city));
        self.row(g, 1, ROWS_TOP + 30.0, row_w, 21.0, "Time zone", &value, true);
        let (a, dx, dy) = self.reveal(4);
        let now = chrono::Local::now().format(if g.clock.0 { "%H:%M" } else { "%-I:%M %p" }).to_string().to_lowercase();
        let offset = crate::device::zone_offset(&zone).map(|o| format!(" {o}")).unwrap_or_default();
        let line = format!("now {now}{offset} · clock: {}", crate::device::clock_note());
        let line = g.text.fit(&line, &TextStyle::code(9.5), row_w - 11.0);
        g.text_alpha(LEFT + 11.0 + dx, ROWS_TOP + 64.0 + dy, &line, TextStyle::code(9.5), INK_THIN, a);
        self.bar(g, "esc back · tab skip");
    }
}

/// An object on its spot, `d` pages along the way from the camera (ahead
/// positive), turned `yaw` and scaled `k` besides. (The spacing, the turn of
/// 55° a page and the growing are our own tuning.)
fn stand(g: &mut Gfx, object: &str, spot: (f32, f32), d: f32, yaw: f32, k: f32) {
    let x = spot.0 + d * SPACING;
    let near = if d > 0.0 { 1.0 - 0.25 * d } else { 1.0 - 0.3 * d };
    let scale = (near * k).clamp(0.5, 1.4);
    // Whole on its spot, gone a page away; one passed goes sooner, before
    // it crosses the words of the page arriving.
    let alpha = (1.0 - if d < 0.0 { -1.5 * d } else { d }).clamp(0.0, 1.0);
    if alpha <= 0.01 {
        return;
    }
    // Setup's ground is dark in either mode, so its objects take the dark
    // palette (with the accent chosen).
    let tokens = g.tokens;
    g.tokens = Tokens::of(g.accent, Tone::Dark);
    let signal = g.tokens.signal;
    g.canvas.glow(x + g.ox, spot.1 + 30.0 * scale, 52.0 * scale, signal, 0.08 * alpha);
    g.object(object, "hero", x, spot.1, crate::object3d::View::yaw(yaw - d * 55.0), scale, alpha);
    g.tokens = tokens;
}

impl Default for Setup {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for Setup {
    fn title(&self) -> &str {
        if self.from_settings && self.page == Page::Lesson {
            "Keyboard lesson"
        } else {
            "Setup"
        }
    }

    fn full_screen(&self) -> bool {
        true
    }

    fn wants_text(&self) -> bool {
        (self.page == Page::Team && self.focus == 0)
            || (self.page == Page::Place && self.focus == 0)
            || (self.page == Page::Lesson && self.lesson.typing())
            || self.typing.is_some()
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn appear(&mut self) {
        self.entered = 0.0;
    }

    fn resume(&mut self) {
        // Back from choosing a network or a time zone: say how it went.
        self.entered = 0.0;
        // Back from joining a network: the network turns once round, and
        // setup carries on by itself (see `update`).
        let online = self.wifi.as_ref().is_some_and(|w| w.state().online()) || crate::sys::address("wlan0").is_some_and(|a| a.contains('.'));
        if self.page == Page::Wifi && online {
            self.focus = self.rows();
            if self.joined.is_none() {
                self.joined = Some(0.0);
                self.celebrate();
            }
        }
        // Back from typing the line: the keys' chip says how it went.
        if let Some(check) = &mut self.hardware {
            check.tick(10.0);
        }
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
                    // A key kept: the key turns once round.
                    if !text.is_empty() {
                        self.celebrate();
                    }
                }
                Key::Back => self.typing = None,
                _ => {}
            }
            return Nav::Stay;
        }
        if self.page == Page::Lesson {
            return self.lesson_key(key, cx);
        }
        // Receiving runs by itself to "all set".
        if self.page == Page::Receiving {
            return Nav::Stay;
        }
        if self.page == Page::Hardware {
            return self.hardware_key(key, cx);
        }
        let (rows, last) = (self.rows(), self.rows() + self.has_next() as usize);
        match key {
            // From Settings, Esc goes back there, and the page stops being offered.
            Key::Back if self.from_settings && self.page == Page::Computer => return Nav::Pop,
            Key::Back if self.page != Page::Welcome => return self.back(),
            Key::Tab if self.has_next() => return self.next(cx),
            Key::Up => self.focus = self.focus.saturating_sub(1),
            Key::Down if self.focus + 1 < last => self.focus += 1,
            // The Next button.
            Key::Enter if self.has_next() && self.focus == rows => return self.next(cx),
            // Each digit typed turns the robot a little, as if it looked up.
            Key::Char(c) if self.page == Page::Team && self.focus == 0 && c.is_ascii_digit() && self.team.len() < 5 => {
                self.team.push(c);
                self.nudge(1.0);
            }
            Key::Backspace if self.page == Page::Team => {
                if self.team.pop().is_some() {
                    self.nudge(-1.0);
                }
            }
            Key::Enter if self.page == Page::Team => return self.next(cx),
            // The name: typed as a network will take it (lower case, a hyphen
            // for a space); Enter goes on to the time zone.
            Key::Char(c) if self.page == Page::Place && self.focus == 0 => crate::device::type_name(&mut self.name, c),
            Key::Backspace if self.page == Page::Place && self.focus == 0 => {
                self.name.pop();
            }
            Key::Enter => match (self.page, self.focus) {
                (Page::Welcome, 0) => return self.next(cx),
                (Page::Welcome, 1) => {
                    self.go(Page::Computer, true);
                    self.remote = crate::remote::Remote::start();
                }
                (Page::Welcome, _) => {
                    crate::mark_setup_done();
                    return Nav::Done;
                }
                // No network yet for the computer page: join Wi-Fi from here.
                (Page::Computer, _) if self.remote.is_none() => {
                    if !crate::wifi::switched_on() {
                        crate::wifi::switch(true);
                    }
                    return Nav::Push(Box::new(crate::screens::WifiScreen::for_setup()));
                }
                (Page::Place, 0) => self.focus = 1,
                (Page::Place, _) => return Nav::Push(Box::new(crate::device::ZonePicker::new())),
                (Page::Wifi, 0) => {
                    if !crate::wifi::switched_on() {
                        crate::wifi::switch(true);
                    }
                    return Nav::Push(Box::new(crate::screens::WifiScreen::for_setup()));
                }
                (Page::Wifi, _) => return self.next(cx),
                (Page::Keys, row) => self.typing = Some((row, String::new())),
                (Page::Choices, row) => {
                    self.toggle_choice(row, cx);
                    self.nudge(1.0);
                }
                (Page::Power, row) => {
                    self.change_power(row, 1, cx);
                    self.nudge(1.0);
                }
                (Page::Look, row) => self.change_look(row, 1, cx),
                _ => {}
            },
            Key::Left | Key::Right if self.page == Page::Power && self.focus < rows => {
                let step = if key == Key::Left { -1 } else { 1 };
                self.change_power(self.focus, step, cx);
                self.nudge(step as f32);
            }
            Key::Left | Key::Right if self.page == Page::Look && self.focus < rows => {
                self.change_look(self.focus, if key == Key::Left { -1 } else { 1 }, cx);
            }
            Key::Left | Key::Right if self.page == Page::Choices && self.focus < rows => {
                self.toggle_choice(self.focus, cx);
                self.nudge(if key == Key::Left { -1.0 } else { 1.0 });
            }
            _ => {}
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        // A network joined: on to the next page by itself, once its object
        // has turned round.
        if self.page == Page::Wifi && self.joined.is_some_and(|at| self.entered - at >= JOINED_HOLD) {
            self.joined = None;
            return self.next(cx);
        }
        let Some(answers) = self.remote.as_mut().and_then(|r| r.take()) else { return Nav::Stay };
        self.take_answers(answers, cx);
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.entered += dt;
        self.clock += dt;
        // The object in front changed on its page: it flips to the new one.
        let hero = self.hero();
        if hero != self.shown {
            if self.calm || self.shown.is_none() || hero.is_none() {
                self.flip.jump(1.0);
                self.flip_from = None;
            } else {
                self.flip_from = self.shown;
                self.flip.jump(0.0);
                self.flip.set(1.0);
            }
            self.shown = hero;
        }
        // Where it rests, the short way round to it.
        let now = self.look.value();
        let want = now + ((self.rest_yaw() - now + 180.0).rem_euclid(360.0) - 180.0);
        if (self.look.target() - want).abs() > 0.01 {
            if self.calm {
                self.look.jump(want);
            } else {
                self.look.set(want);
            }
        }
        let objects = self.travel.tick(dt) | self.spin.tick(dt) | self.flip.tick(dt) | self.look.tick(dt) | (self.page == Page::Finishing) | self.joined.is_some();
        // A page's elements have all arrived within about half a second, and
        // the last page runs on its own; otherwise it is drawn again only
        // when what it shows has changed: the computer page's status, the
        // Wi-Fi page's connection. The lesson's first card blinks its
        // lights, and a key just pressed on the second fades.
        let mut changed = self.entered < 0.8 || objects;
        if self.page == Page::Lesson {
            changed |= self.lesson.card() == Card::Modifiers || self.lesson.pressed.iter().any(|p| self.clock - p < 0.4);
        }
        if let Some(remote) = &mut self.remote {
            changed |= remote.tick();
        } else if self.page == Page::Computer {
            // Waiting for a network (Wi-Fi joined, or a cable): the page is
            // offered as soon as there is an address to give.
            self.looked_for_network += dt;
            if self.looked_for_network >= 2.0 {
                self.looked_for_network = 0.0;
                self.remote = crate::remote::Remote::start();
                changed |= self.remote.is_some();
            }
        }
        if let Some(wifi) = &self.wifi {
            let seen = wifi.generation();
            changed |= std::mem::replace(&mut self.wifi_seen, seen) != seen;
        }
        // What a computer sent, arriving: the list scrolls to keep the
        // newest row in view; once all are in and have shown, all set.
        if self.page == Page::Receiving {
            let shown = self.received_shown();
            let target = shown.saturating_sub(RECEIVE_ROWS) as f32;
            if self.received_scroll.target() != target {
                self.received_scroll.set(target);
            }
            self.received_scroll.tick(dt);
            if self.entered >= RECEIVE_FIRST + self.received.len() as f32 * RECEIVE_STEP + RECEIVE_HOLD {
                // On to what is left for the handheld, else all set.
                let next = if self.sent_from_computer { self.flow.first().copied() } else { None };
                self.go(next.unwrap_or(Page::Finishing), true);
            }
            changed = true;
        }
        // The parts looked for again now and then (the motion sensor starts a
        // moment after the shell; the Ethernet adapter follows the USB switch).
        if self.page == Page::Hardware {
            if let Some(check) = &mut self.hardware {
                changed |= check.tick(dt);
            }
        }
        changed
    }

    fn busy(&self) -> bool {
        matches!(self.page, Page::Finishing | Page::Receiving) || (self.page == Page::Lesson && self.lesson.card() == Card::Modifiers)
    }

    fn finished(&self) -> bool {
        self.page == Page::Finishing && self.entered >= ALL_SET
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        self.calm = g.calm;
        g.canvas.px.copy_from_slice(&self.ground);
        self.draw_objects(g);
        self.header(g);
        let row_w = self.column();
        match self.page {
            Page::Welcome => {
                let (a, dx, dy) = self.reveal(0);
                g.text_alpha(LEFT + dx, TITLE_CY + 2.0 + dy, "Hi.", TextStyle::display(26.0), INK_STRONG, a);
                // The line under it says where the focused way leads.
                let (a, dx, dy) = self.reveal(1);
                let words = WAYS.get(self.focus).map_or("", |w| w.1);
                g.text_alpha(LEFT + dx, WORDS_CY + 4.0 + dy, words, TextStyle::sans(11.0, 400.0), INK_THIN, a);
                for (i, (label, _)) in WAYS.iter().enumerate() {
                    self.row(g, i, ROWS_TOP + 6.0 + i as f32 * 24.0, row_w, 21.0, label, "", false);
                }
                self.bar(g, "enter chooses");
            }
            Page::Computer => {
                self.heading(g, "From a computer", if self.remote.is_some() { WORDS_COMPUTER } else { WORDS_NO_NETWORK });
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
                        let _ = (a, dx, dy);
                        self.row(g, 0, ROWS_TOP + 6.0, row_w, 21.0, "Connect to Wi-Fi", "›", false);
                        let (a, dx, dy) = self.reveal(3);
                        let small = TextStyle::sans(11.0, 400.0);
                        g.text_alpha(LEFT + 11.0 + dx, ROWS_TOP + 46.0 + dy, WORDS_OR_CABLE, small, INK_THIN, a);
                        g.text_alpha(LEFT + 11.0 + dx, ROWS_TOP + 62.0 + dy, WORDS_OPENS_ITSELF, small, INK_THIN, a);
                        self.bar(g, "enter: wi-fi · esc back");
                    }
                }
            }
            Page::Lesson => self.draw_lesson(g),
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
                    let line = format!("robot at {}", crate::screens::resolve_address(&team.to_string()));
                    g.text_alpha(LEFT + 2.0 + dx, field.bottom() + 14.0 + dy, &line, TextStyle::code(9.5), INK_THIN, a);
                }
                self.bar(g, "esc back · tab skip");
            }
            Page::Place => self.draw_place(g),
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
                self.heading(g, "Keys for services", words);
                for (i, (name, label)) in SERVICES.iter().take(crate::keys::SETUP_SERVICES).enumerate() {
                    let value = match &self.typing {
                        // The end of what is typed, as much as fits beside the service's name.
                        Some((row, text)) if *row == i => {
                            let room = row_w - 33.0 - g.measure(label, TextStyle::sans(11.5, 600.0));
                            let chars: Vec<char> = text.chars().collect();
                            let mut take = chars.len().min(18);
                            let tail = |n: usize| format!("{}▏", chars[chars.len() - n..].iter().collect::<String>());
                            while take > 0 && g.measure(&tail(take), TextStyle::code(9.5)) > room {
                                take -= 1;
                            }
                            tail(take)
                        }
                        _ => self.keys.shown(name),
                    };
                    // Four rows, spaced as Choices' four are.
                    self.row(g, i, ROWS_TOP + i as f32 * 20.0, row_w, 17.0, label, &value, self.keys.get(name).is_some());
                }
                self.bar(g, "esc back · tab skip");
            }
            Page::Choices => {
                let on = [g.open_robot, g.match_report, crate::ssh_enabled(), g.calm];
                // The line under the title explains whichever choice is focused.
                let words = CHOICES.get(self.focus).map_or(WORDS_DEFAULTS, |c| c.1);
                self.heading(g, "Choose your settings", words);
                for (i, ((label, _), on)) in CHOICES.iter().zip(on).enumerate() {
                    self.row(g, i, ROWS_TOP + i as f32 * 20.0, row_w, 17.0, label, if on { "on" } else { "off" }, on);
                }
                self.bar(g, if self.focus == self.rows() { "enter: next · ↑ to change" } else { "enter switches · esc back" });
            }
            Page::Hardware => self.draw_hardware(g),
            Page::Receiving => self.draw_receiving(g),
            Page::Power => {
                let words = POWER.get(self.focus).map_or(WORDS_DEFAULTS, |p| p.1);
                self.heading(g, "Battery and power", words);
                let on = [g.screen_off != 0, g.battery.saver_auto, g.battery.wifi_sleeps, g.lift_wake];
                for (i, ((label, _), on)) in POWER.iter().zip(on).enumerate().take(self.power_rows()) {
                    let value = match i {
                        0 => format!("‹ {} ›", crate::standby::screen_off_name(g.screen_off)),
                        _ => (if on { "on" } else { "off" }).to_owned(),
                    };
                    self.row(g, i, ROWS_TOP + i as f32 * 20.0, row_w, 17.0, label, &value, on);
                }
                self.bar(g, if self.focus == self.rows() { "enter: next · ↑ to change" } else { "enter switches · ← → time · esc back" });
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
                self.bar(g, if self.focus == self.rows() { "enter: finish · ↑ to change" } else { "← → change · esc back" });
            }
            Page::Finishing => {
                let done = self.entered >= WORKING;
                let next = if self.from_settings { "Back to Settings." } else { "Home is next." };
                let (title, words) = if done { ("You're all set.", next) } else { ("Just a moment…", "Putting your choices in place.") };
                // The handheld in the middle, turning while it works; then
                // the trophy turns in (`draw_objects`). Calm, the handheld
                // stands still, so a small arc says it is working.
                let style = TextStyle::sans(17.0, 600.0);
                let tw = g.measure(title, style);
                g.text_alpha((W - tw) * 0.5, 120.0, title, style, INK_STRONG, 1.0);
                let small = TextStyle::sans(11.0, 400.0);
                let ww = g.measure(words, small);
                g.text_alpha((W - ww) * 0.5, 137.0, words, small, INK_THIN, 1.0);
                if !done && g.calm {
                    let spin = self.entered * 5.2;
                    g.canvas.arc(W * 0.5, 156.0, 6.0, 2.0, spin, 1.4 + 0.8 * (self.entered * 3.1).sin(), CRIMSON, 1.0);
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
        // Pages laid out across the panel.
        for line in [WORDS_COMPUTER, WORDS_LOOK, WORDS_HARDWARE] {
            let w = g.measure(line, words);
            assert!(w <= W - 2.0 * LEFT, "{line:?} is {w:.0} px, wider than {}", W - 2.0 * LEFT);
        }
        // Pages with their object beside them: the words, and every title, in the column left of it.
        let beside = [WORDS_NO_NETWORK, WORDS_OR_CABLE, WORDS_OPENS_ITSELF, WORDS_TEAM, WORDS_PLACE, WORDS_WIFI, WORDS_KEYS, WORDS_TYPING, WORDS_DEFAULTS];
        // (Every one too wide is listed, not just the first.)
        let mut wide = Vec::new();
        let ways = WAYS.iter().map(|w| w.1);
        for line in beside.into_iter().chain(ways).chain(CHOICES.iter().map(|c| c.1)).chain(POWER.iter().map(|p| p.1)) {
            let w = g.measure(line, words);
            if w > COLUMN {
                wide.push(format!("{line:?} is {w:.0} px"));
            }
        }
        let title = TextStyle::sans(18.0, 600.0);
        for t in ["Connect to Wi-Fi?", "Which team is it for?", "Name and time zone", "Keys for services", "Choose your settings", "Battery and power", "From a computer"] {
            let w = g.measure(t, title);
            if w > COLUMN {
                wide.push(format!("{t:?} is {w:.0} px"));
            }
        }
        // The choices' names fit their rows beside "off", the power page's
        // beside its longest value, the welcome's ways on their own.
        let label = TextStyle::sans(11.5, 600.0);
        for (name, _) in CHOICES.iter().chain(WAYS.iter()) {
            let w = g.measure(name, label) + 22.0 + g.measure("off", TextStyle::code(9.5)) + 11.0;
            if w > COLUMN {
                wide.push(format!("{name:?} crowds its row: {w:.0} px"));
            }
        }
        let longest = crate::standby::SCREEN_OFF_CHOICES.iter().map(|v| g.measure(&format!("‹ {} ›", crate::standby::screen_off_name(*v)), TextStyle::code(9.5))).fold(0.0, f32::max);
        for (name, _) in POWER {
            let w = g.measure(name, label) + 22.0 + longest + 11.0;
            if w > COLUMN {
                wide.push(format!("{name:?} crowds its row: {w:.0} px"));
            }
        }
        // The keys' rows: each service beside a key shown by its end.
        for (_, service) in SERVICES.iter().take(crate::keys::SETUP_SERVICES) {
            let w = g.measure(service, label) + 22.0 + g.measure("set · …abcd", TextStyle::code(9.5)) + 11.0;
            if w > COLUMN {
                wide.push(format!("{service:?} crowds its row: {w:.0} px"));
            }
        }
        assert!(wide.is_empty(), "wider than the column's {COLUMN}: {wide:#?}");
        // Each part's chip on the hardware page, and what the line says of it.
        let cw = (W - 2.0 * LEFT - 5.0 * (CHIPS_ACROSS - 1) as f32) / CHIPS_ACROSS as f32;
        for part in crate::hwcheck::PARTS {
            let w = g.measure(part.short(), TextStyle::sans(10.5, 600.0));
            assert!(w <= cw - 24.0, "{:?} crowds its chip: {w:.0} px", part.short());
            for state in [crate::hwcheck::look(part, false), crate::hwcheck::look(part, true)] {
                let line = format!("{}: {}", part.name(), state.words());
                assert!(g.measure(&line, words) <= W - 2.0 * LEFT, "{line:?}");
            }
        }
        // Rows of chips end above the bar.
        let rows = crate::hwcheck::PARTS.len().div_ceil(CHIPS_ACROSS) as f32;
        assert!(ROWS_TOP + 1.0 + (rows - 1.0) * 21.0 + 18.0 <= BAR_CY - 9.0);
    }

    /// Setup in a sandbox, past the welcome and onto the lesson's first card.
    fn on_the_lesson() -> (Setup, crate::shell::Settings, Robot) {
        crate::shell::tests::sandbox();
        let mut setup = Setup::new();
        setup.go(Page::Lesson, true);
        (setup, crate::shell::Settings::default(), Robot::none())
    }

    fn press(setup: &mut Setup, settings: &mut crate::shell::Settings, robot: &mut Robot, key: Key) -> Nav {
        let mut say = None;
        let mut cx = Cx { robot, settings, say: &mut say, at_rest: true };
        setup.key(key, &mut cx)
    }

    #[test]
    fn what_a_computer_sent_arrives_row_by_row_then_all_is_set() {
        let sent = crate::remote::answers(&crate::remote::parse_form("team=5805&ssid1=Workshop&psk1=hunter22&tba=abcdefgh12345678&accent=rose&mode=light&choices=1&open_robot=on"));
        let mut setup = Setup::showing_received(&sent);
        let names: Vec<&str> = setup.received.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Team", "Wi-Fi network", "The Blue Alliance key", "Look", "Choices"]);
        assert_eq!(setup.received[3].1, "rose · light");
        assert!(!setup.received[2].1.contains("abcdefgh"), "a key shows only its end: {}", setup.received[2].1);
        assert_eq!(setup.received_shown(), 0);
        setup.tick(RECEIVE_FIRST + RECEIVE_STEP * 1.5);
        assert_eq!(setup.received_shown(), 2, "a row at a time");
        let mut waited = 0.0;
        while setup.page == Page::Receiving && waited < 10.0 {
            setup.tick(0.1);
            waited += 0.1;
        }
        assert_eq!(setup.page, Page::Finishing, "then all set, by itself");
        // Nothing filled in is said so.
        assert_eq!(crate::remote::received(&crate::remote::Answers::default()), [("Nothing to change".to_owned(), String::new())]);
        // Every row fits beside its value.
        let mut g = Gfx::new(Tokens::dark());
        let full = crate::remote::answers(&crate::remote::parse_form("team=99999&ssid1=A%20rather%20long%20network%20name&tba=x123456789&nexus=y123456789&openai=z123456789&claude=sk-ant-w123456789&ssh_key=ssh-ed25519%20AAA&accent=rose&mode=light&text=large&home=carousel&choices=1"));
        for (name, _) in crate::remote::received(&full) {
            assert!(g.measure(&name, TextStyle::sans(11.0, 600.0)) + 32.0 + 60.0 <= W - 2.0 * LEFT, "{name}");
        }
    }

    #[test]
    fn after_a_computer_sends_on_first_start_the_handheld_pages_follow() {
        crate::shell::tests::sandbox();
        let (mut settings, mut robot) = (crate::shell::Settings::default(), Robot::none());
        // First start, set up from a computer that sent the team, a network,
        // the look and the choices.
        let mut setup = Setup::new();
        setup.go(Page::Computer, true);
        let sent = crate::remote::answers(&crate::remote::parse_form("team=5805&ssid1=Workshop&psk1=hunter22&accent=rose&mode=light&choices=1&open_robot=on"));
        let mut say = None;
        setup.take_answers(sent, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        assert_eq!(setup.page, Page::Receiving);
        assert_eq!(setup.flow, [Page::Hardware, Page::Lesson, Page::Place, Page::Power], "what the computer page has nothing for");
        // What arrived shows, then on to the hardware check: not all set yet.
        let mut waited = 0.0;
        while setup.page == Page::Receiving && waited < 10.0 {
            setup.tick(0.1);
            waited += 0.1;
        }
        assert_eq!(setup.page, Page::Hardware);
        // Nothing to go back to: what was sent is in place.
        press(&mut setup, &mut settings, &mut robot, Key::Back);
        assert_eq!(setup.page, Page::Hardware);
        // Through them in order (Tab skips each, the lesson whole), then all set.
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(setup.page);
            press(&mut setup, &mut settings, &mut robot, Key::Tab);
        }
        assert_eq!(seen, [Page::Hardware, Page::Lesson, Page::Place, Page::Power]);
        assert_eq!(setup.page, Page::Finishing);
        // A computer that left the team, the look and the choices unanswered:
        // those are asked here too. Wi-Fi and the keys never are.
        assert_eq!(
            left_for_handheld(&crate::remote::Answers::default()),
            [Page::Hardware, Page::Lesson, Page::Team, Page::Place, Page::Choices, Page::Power, Page::Look]
        );
        // From Settings, receiving is the whole job.
        let sent = crate::remote::answers(&crate::remote::parse_form("team=5805"));
        let mut again = Setup { from_settings: true, ..Setup::new() };
        again.go(Page::Computer, true);
        let mut say = None;
        again.take_answers(sent, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        let mut waited = 0.0;
        while again.page == Page::Receiving && waited < 10.0 {
            again.tick(0.1);
            waited += 0.1;
        }
        assert_eq!(again.page, Page::Finishing);
    }

    #[test]
    fn setup_goes_through_its_pages_in_order_and_back() {
        let (mut setup, mut settings, mut robot) = on_the_lesson();
        setup.go(Page::Welcome, true);
        let mut seen = Vec::new();
        press(&mut setup, &mut settings, &mut robot, Key::Enter);
        for _ in 0..FLOW.len() {
            seen.push(setup.page);
            // Tab skips each page, the lesson whole.
            press(&mut setup, &mut settings, &mut robot, Key::Tab);
        }
        assert_eq!(seen, FLOW.to_vec());
        assert_eq!(setup.page, Page::Finishing, "the last page finishes");
        // And back, a page at a time, from the end to the welcome.
        let mut setup = Setup::new();
        setup.go(*FLOW.last().unwrap(), true);
        for want in FLOW.iter().rev().skip(1) {
            press(&mut setup, &mut settings, &mut robot, Key::Back);
            assert_eq!(setup.page, *want);
            // The lesson, arrived at from behind, is on its last card: back through its cards.
            if *want == Page::Lesson {
                for _ in 1..CARDS.len() {
                    press(&mut setup, &mut settings, &mut robot, Key::Back);
                    assert_eq!(setup.page, Page::Lesson);
                }
            }
        }
        press(&mut setup, &mut settings, &mut robot, Key::Back);
        assert_eq!(setup.page, Page::Welcome);
    }

    #[test]
    fn pages_of_choices_already_made_start_on_next_so_enter_goes_on() {
        let (mut setup, mut settings, mut robot) = on_the_lesson();
        for page in [Page::Keys, Page::Choices, Page::Power, Page::Look, Page::Hardware] {
            setup.go(page, true);
            assert_eq!(setup.focus, setup.rows(), "{page:?} starts on Next");
        }
        // Enter, Enter, Enter: through keys, choices and power to the look.
        setup.go(Page::Keys, true);
        for _ in 0..3 {
            press(&mut setup, &mut settings, &mut robot, Key::Enter);
        }
        assert_eq!(setup.page, Page::Look);
        // Up goes into the rows.
        press(&mut setup, &mut settings, &mut robot, Key::Up);
        assert_eq!(setup.focus, setup.rows() - 1);
        // A page asking for something starts on it.
        setup.go(Page::Team, true);
        assert_eq!(setup.focus, 0);
    }

    #[test]
    fn once_wi_fi_joins_setup_carries_on_by_itself() {
        let (mut setup, mut settings, mut robot) = on_the_lesson();
        setup.go(Page::Wifi, true);
        // Joined, back from the list of networks.
        setup.joined = Some(0.0);
        setup.celebrate();
        assert!(setup.spin.value() < -300.0, "its object turns once round: {}", setup.spin.value());
        let mut say = None;
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        setup.tick(0.5);
        assert!(matches!(setup.update(&mut cx), Nav::Stay));
        assert_eq!(setup.page, Page::Wifi, "while it turns, the page stays");
        setup.tick(JOINED_HOLD);
        setup.update(&mut cx);
        assert_eq!(setup.page, Page::Hardware, "then on by itself");
    }

    #[test]
    fn each_page_has_its_object_and_the_way_through_moves_between_them() {
        let (mut setup, ..) = on_the_lesson();
        setup.go(Page::Welcome, true);
        assert_eq!(setup.hero(), Some("handheld"));
        setup.focus = 1;
        assert_eq!(setup.hero(), Some("laptop"), "from a computer");
        setup.tick(0.01);
        assert_eq!(setup.flip_from, Some("handheld"), "the handheld turns away for it");
        setup.focus = 0;
        let pages = [(Page::Wifi, "wifi"), (Page::Team, "robot"), (Page::Place, "globe"), (Page::Keys, "key"), (Page::Choices, "toggle"), (Page::Power, "battery")];
        for (page, object) in pages {
            setup.go(page, true);
            assert_eq!(setup.hero(), Some(object), "{page:?}");
            assert!(crate::object3d::Objects::shipped().has(object), "{object} is in the pack");
        }
        for page in [Page::Hardware, Page::Lesson, Page::Look] {
            setup.go(page, true);
            assert_eq!(setup.hero(), None, "{page:?} is laid out across the panel");
        }
        // On from the team to the place: the robot is carried off, the camera glides on.
        setup.go(Page::Team, true);
        for _ in 0..80 {
            setup.tick(0.025);
        }
        let at = setup.travel.value();
        setup.go(Page::Place, true);
        assert_eq!(setup.leaving.map(|l| l.0), Some("robot"));
        setup.tick(0.05);
        assert!(setup.travel.value() > at && setup.travel.value() < at + 1.0, "on its way");
        // Calm: there at once, nothing carried.
        setup.calm = true;
        setup.go(Page::Keys, true);
        assert_eq!(setup.travel.value(), setup.slot());
        assert!(setup.leaving.is_none());
        // The last page: the handheld at work, then the trophy.
        setup.go(Page::Finishing, true);
        assert_eq!(setup.hero(), Some("handheld"));
        setup.tick(WORKING + 0.1);
        assert_eq!(setup.hero(), Some("trophy"));
    }

    #[test]
    fn the_globe_turns_to_the_time_zone() {
        assert_eq!(offset_hours("UTC"), 0.0);
        assert_eq!(offset_hours("UTC−5"), -5.0);
        assert_eq!(offset_hours("UTC+8"), 8.0);
        assert_eq!(offset_hours("UTC+5:30"), 5.5);
        assert_eq!(offset_hours("UTC-3:30"), -3.5);
    }

    #[test]
    fn the_lesson_never_holds_anyone_up() {
        let (mut setup, mut settings, mut robot) = on_the_lesson();
        // Enter goes on through every card, practised or not, and on to the team.
        for card in CARDS {
            assert_eq!(setup.lesson.card(), card);
            press(&mut setup, &mut settings, &mut robot, Key::Enter);
        }
        assert_eq!(setup.page, Page::Team);
    }

    #[test]
    fn the_lesson_ticks_off_what_arrives() {
        let (mut setup, mut settings, mut robot) = on_the_lesson();
        press(&mut setup, &mut settings, &mut robot, Key::Enter);
        // Moving: the arrows (F X Z C off the handheld's keys, or a computer's arrows).
        assert!(!setup.wants_text(), "outside typing, so F X Z C arrive as arrows");
        for key in [Key::Up, Key::Down, Key::Left] {
            press(&mut setup, &mut settings, &mut robot, key);
        }
        assert!(!setup.lesson.done());
        press(&mut setup, &mut settings, &mut robot, Key::Right);
        assert!(setup.lesson.done());
        assert_eq!(setup.lesson.at, (1, 1), "the ring went down, left, then right");
        // Capitals: a capital and a symbol, typed.
        press(&mut setup, &mut settings, &mut robot, Key::Enter);
        assert!(setup.wants_text(), "a text field: the letters type");
        for key in [Key::Char('g'), Key::Char('O')] {
            press(&mut setup, &mut settings, &mut robot, key);
        }
        assert!(setup.lesson.capital && !setup.lesson.symbol);
        press(&mut setup, &mut settings, &mut robot, Key::Char('!'));
        assert!(setup.lesson.done());
        assert_eq!(setup.lesson.text, "gO!");
        // Arrows while typing: an F types an f; an arrow moves the cursor.
        press(&mut setup, &mut settings, &mut robot, Key::Enter);
        assert_eq!(setup.lesson.text, "catalyst");
        press(&mut setup, &mut settings, &mut robot, Key::Char('f'));
        assert!(!setup.lesson.done(), "a letter typed is not the cursor moving");
        press(&mut setup, &mut settings, &mut robot, Key::Left);
        press(&mut setup, &mut settings, &mut robot, Key::Char('x'));
        assert_eq!(setup.lesson.text, "catalystxf");
        assert!(setup.lesson.done());
        // Fn's keys, lit as they are tried.
        press(&mut setup, &mut settings, &mut robot, Key::Enter);
        press(&mut setup, &mut settings, &mut robot, Key::F(10));
        press(&mut setup, &mut settings, &mut robot, Key::End);
        assert_eq!(setup.lesson.tried, [false, true, false, true, false, false]);
    }

    #[test]
    fn the_lesson_from_settings_comes_back_to_settings() {
        crate::shell::tests::sandbox();
        let (mut settings, mut robot) = (crate::shell::Settings::default(), Robot::none());
        let mut lesson = Setup::lesson();
        assert_eq!(lesson.title(), "Keyboard lesson");
        assert!(matches!(press(&mut lesson, &mut settings, &mut robot, Key::Back), Nav::Pop), "esc on the first card");
        let mut lesson = Setup::lesson();
        assert!(matches!(press(&mut lesson, &mut settings, &mut robot, Key::Tab), Nav::Pop), "tab skips it");
        let mut lesson = Setup::lesson();
        let mut nav = Nav::Stay;
        for _ in 0..CARDS.len() {
            nav = press(&mut lesson, &mut settings, &mut robot, Key::Enter);
        }
        assert!(matches!(nav, Nav::Pop), "enter on the last card is done");
    }

    #[test]
    fn the_name_is_typed_as_a_network_takes_it() {
        let (mut setup, mut settings, mut robot) = on_the_lesson();
        setup.go(Page::Place, true);
        setup.name.clear();
        assert!(setup.wants_text());
        for c in " Pit Crew!".chars() {
            press(&mut setup, &mut settings, &mut robot, Key::Char(c));
        }
        assert_eq!(setup.name, "pit-crew", "lower case, a hyphen for the space, nothing leading");
        // Enter goes on to the time zone, where the letters move again.
        press(&mut setup, &mut settings, &mut robot, Key::Enter);
        assert_eq!(setup.focus, 1);
        assert!(!setup.wants_text());
        assert!(matches!(press(&mut setup, &mut settings, &mut robot, Key::Enter), Nav::Push(_)), "the time zones");
    }

    #[test]
    fn every_lesson_card_fits_the_panel_whole() {
        let mut g = Gfx::new(Tokens::dark());
        let mut setup = Setup::lesson();
        let words = TextStyle::sans(11.0, 400.0);
        for i in 0..CARDS.len() {
            setup.lesson.arrive(i);
            setup.draw(&mut g, &Robot::none());
        }
        for (title, line) in CARD_WORDS {
            let w = g.measure(line, words);
            assert!(w <= W - 2.0 * LEFT, "{line:?} is {w:.0} px");
            assert!(g.measure(title, TextStyle::sans(18.0, 600.0)) <= W - 2.0 * LEFT, "{title:?}");
        }
        // The ways of a modifier beside their light and name.
        for (how, what, _) in MODIFIER_WAYS {
            assert!(g.measure(how, TextStyle::sans(11.5, 600.0)) <= 92.0 - 26.0 - 4.0, "{how}");
            assert!(92.0 + g.measure(what, TextStyle::sans(10.5, 400.0)) <= W - 2.0 * LEFT - 8.0, "{what}");
        }
        // Fn's keys in two columns.
        let col_w = (W - 2.0 * LEFT - 6.0) * 0.5;
        for (keys, what, _) in FN_KEYS {
            let w = 4.0 + g.measure(keys, TextStyle::code(9.0)) + 12.0 + 7.0 + g.measure(what, TextStyle::sans(10.5, 600.0)) + 6.0;
            assert!(w <= col_w, "{keys} {what}: {w:.0} of {col_w:.0}");
        }
        // The practice tasks on one line, with room to spare.
        let task = |g: &mut Gfx, s: &str| g.measure(s, TextStyle::sans(10.5, 600.0)) + 30.0;
        let capitals = task(&mut g, TASK_CAPITAL) + 6.0 + task(&mut g, TASK_SYMBOL);
        assert!(capitals <= W - 2.0 * LEFT, "capitals' tasks: {capitals:.0}");
        let moving: f32 = ["up", "down", "left", "right"].iter().map(|s| task(&mut g, s) + 5.0).sum();
        assert!(moving <= W - 2.0 * LEFT, "moving's tasks: {moving:.0}");
        assert!(task(&mut g, TASK_CURSOR) <= W - 2.0 * LEFT);
        // The hints along the bottom, before the lesson's dots.
        for hint in ["enter: on to setup · esc: back", "enter: next · esc: back · tab: skip", "enter: next · tab: skip the lesson"] {
            let w = g.measure(hint, TextStyle::code(9.0));
            assert!(LEFT + w + 8.0 <= W - LEFT - (CARDS.len() as f32 - 1.0) * 10.0 - 6.0, "{hint:?}: {w:.0}");
        }
    }
}
