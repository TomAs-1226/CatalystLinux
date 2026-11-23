//! The lock screen. With no passcode (the default) there is none: a key
//! wakes the handheld straight to where it was. With one set (Settings ›
//! Lock screen), once the screen has gone off (or StandBy has come up on the
//! charger, which stands in for it), the next wake shows the lock screen,
//! and only the passcode takes it away.
//!
//! - **Nothing gets round it.** The lock is a layer over the whole shell,
//!   and it takes every key before anything else can: the recent apps, the
//!   control centre, Spotlight, `?` and every other shortcut are reached
//!   only from below it, in `Shell::key`. Nothing else is drawn while it
//!   covers the panel, no external app is handed the panel, and the island
//!   says nothing over it. It engages at start too, so a restart (or the
//!   switch off and on) comes back locked.
//! - **The passcode** is checked against its Argon2id hash (passcode.rs) on
//!   a thread of its own. A wrong one shakes the field (a spring kicked
//!   sideways, five visible swings) and tints its rim; from the fifth wrong
//!   one in a row each try waits longer, even across a restart.
//! - **Notifications** not yet seen show on it, newest first, as many as
//!   fit (notify.rs). By default their words wait until it is unlocked
//!   (Apple's "when unlocked", and the safe choice for a handheld on a desk
//!   in the pits): each says only its app, and when; Settings › Lock screen
//!   can show the words. ↑ ↓ pick one, and once unlocked it opens.
//! - **Its look** is the user's own: Settings › Lock screen › Customise
//!   picks the clock (centre, left, dots), its colour, and what shows with
//!   it (the date, the charge, the next match, notices, the next reminder),
//!   with the lock screen itself as the live preview.
//!
//! What a passcode does not protect, as Settings says it: SSH, the setup page
//! a computer opens, and the card itself (taken out and read anywhere).

use bezel::{Align, Color, GlassShape, Rect, Tokens};
use chrono::NaiveDateTime;
use detent::{role, Animated, Spring};

use crate::faces::{chip, chip_width, date_text, draw_dots, short_date, time_text, Tab};
use crate::gfx::{Gfx, H, PAD, W};
use crate::passcode::{self, Tries};
use crate::platform::Key;
use crate::power::Reading;

// --------------------------------------------------------------- choices ---

macro_rules! choice {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn name(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|v| v.name() == name)
            }
            pub fn index(self) -> usize {
                Self::ALL.iter().position(|v| *v == self).unwrap_or(0)
            }
            /// The next one along, or the one before (`step` −1), round.
            pub fn step(self, step: i32) -> Self {
                Self::ALL[(self.index() as i32 + step).rem_euclid(Self::ALL.len() as i32) as usize]
            }
        }
    };
}

choice!(Face { Clock => "clock", Music => "now playing", Charge => "charge", Match => "next match", Weather => "weather", Photos => "photos", Everything => "everything" });
choice!(ClockStyle { Digits => "digits", Dial => "dial", Dots => "dots", Robot => "robot" });
choice!(LockFace { Centre => "centre", Stacked => "stacked", Dots => "dots" });
choice!(LockColour { White => "white", Ice => "ice", Amber => "amber", Accent => "accent" });
choice!(Show { Date => "date", Charge => "charge", Match => "next match", Notices => "notices", Reminder => "next reminder" });

impl LockColour {
    pub fn color(self, t: &Tokens) -> Color {
        match self {
            LockColour::White => t.ink,
            LockColour::Ice => t.ice,
            LockColour::Amber => t.amber_ink,
            LockColour::Accent => t.signal,
        }
    }
}

/// The lock screen's and StandBy's choices, kept with the settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Choices {
    /// StandBy on the charger.
    pub standby: bool,
    /// StandBy turns red and dims from 10 pm to 7 am.
    pub night: bool,
    /// The face StandBy shows first, and the clock's style.
    pub face: Face,
    pub clock: ClockStyle,
    pub lock_face: LockFace,
    pub colour: LockColour,
    /// What shows with the time, a bit for each of `Show::ALL`.
    pub shows: u8,
    /// Notices' words on the lock screen (otherwise only how many).
    pub previews: bool,
    /// The robot stands in front of the time (depth.rs), on the centred
    /// and dotted clocks; and its pose follows the charge and the next match.
    pub depth: bool,
    pub follows: bool,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            standby: true,
            night: true,
            face: Face::Clock,
            clock: ClockStyle::Digits,
            lock_face: LockFace::Centre,
            colour: LockColour::White,
            shows: [Show::Date, Show::Charge, Show::Match, Show::Notices, Show::Reminder].iter().fold(0, |m, s| m | 1 << s.index()),
            previews: false,
            depth: true,
            follows: true,
        }
    }
}

impl Choices {
    pub fn shows(&self, what: Show) -> bool {
        self.shows & (1 << what.index()) != 0
    }

    pub fn toggle(&mut self, what: Show) {
        self.shows ^= 1 << what.index();
    }

    /// As the settings file keeps them.
    pub fn lines(&self) -> [(&'static str, String); 10] {
        let shows: Vec<&str> = Show::ALL.iter().filter(|s| self.shows(**s)).map(|s| s.name()).collect();
        [
            ("standby", self.standby.to_string()),
            ("standby_night", self.night.to_string()),
            ("standby_face", self.face.name().to_owned()),
            ("standby_clock", self.clock.name().to_owned()),
            ("lock_face", self.lock_face.name().to_owned()),
            ("lock_colour", self.colour.name().to_owned()),
            ("lock_shows", shows.join(",")),
            ("lock_previews", self.previews.to_string()),
            ("lock_depth", self.depth.to_string()),
            ("lock_pose_follows", self.follows.to_string()),
        ]
    }

    /// The robot stands in front of the time on this face (the stacked
    /// clock keeps the right for the date and the chips).
    pub fn depth_on(&self) -> bool {
        self.depth && matches!(self.lock_face, LockFace::Centre | LockFace::Dots)
    }

    /// One line of the settings file, if it is one of these.
    pub fn parse(&mut self, key: &str, value: &str) -> bool {
        match key {
            "standby" => self.standby = value != "false",
            "standby_night" => self.night = value != "false",
            "standby_face" => self.face = Face::from_name(value).unwrap_or(self.face),
            "standby_clock" => self.clock = ClockStyle::from_name(value).unwrap_or(self.clock),
            "lock_face" => self.lock_face = LockFace::from_name(value).unwrap_or(self.lock_face),
            "lock_colour" => self.colour = LockColour::from_name(value).unwrap_or(self.colour),
            "lock_shows" => self.shows = value.split(',').filter_map(|n| Show::from_name(n.trim())).fold(0, |m, s| m | 1 << s.index()),
            "lock_previews" => self.previews = value == "true",
            "lock_depth" => self.depth = value != "false",
            "lock_pose_follows" => self.follows = value != "false",
            _ => return false,
        }
        true
    }
}

// ------------------------------------------------------------- the field ---

/// The shake after a wrong passcode: a spring kicked sideways, five visible
/// swings, settled in about 0.6 s. My own tuning (Detent has no preset for
/// it): stiffness 1800, damping ratio 0.26, kicked at 420 px/s (about 8 px).
fn shake_spring() -> Spring {
    Spring::stiffness_ratio(1800.0, 0.26)
}
const SHAKE_KICK: f32 = 420.0;

/// What a key did to the field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    Typed,
    Submit,
    Nothing,
}

/// How the field stands, for its words when it is empty.
#[derive(Clone, Debug, PartialEq)]
pub enum Prompt {
    /// Waiting for the passcode, with these words.
    Asks(String),
    /// Checking what was typed.
    Checking,
    /// The last one was wrong: these words, in the fault colour.
    Wrong(String),
    /// Too many wrong: a wait, counted down.
    Wait(u64),
    /// A warning that outranks the lock (the battery running out).
    Alert(String),
    /// The right one: open, as the lock screen slides away.
    Open,
    /// Shown by hand with no passcode: any key goes back.
    Curtain,
}

/// The field's height, where it sits, and its least width.
pub const FIELD_H: f32 = 28.0;
pub const FIELD_Y: f32 = H - 40.0;
const FIELD_W: f32 = 184.0;

/// The passcode's entry field: a glass capsule whose dots spring in as keys
/// land, shake when the passcode is wrong, and count down a wait. It widens
/// to say what it has to (as the island does), and narrows again after.
pub struct Field {
    text: String,
    /// How many dots show, gliding to how many there are typed: each dot
    /// grows in as the count passes it, a touch past full and back.
    count: Animated,
    shake: Animated,
    /// The rim's fault tint after a wrong one, fading.
    fault: Animated,
    width: Animated,
    placed: bool,
}

// What was typed and never taken is wiped as the field goes.
impl Drop for Field {
    fn drop(&mut self) {
        passcode::wipe(&mut self.text);
    }
}

impl Default for Field {
    fn default() -> Self {
        Self::new()
    }
}

impl Field {
    pub fn new() -> Field {
        Field {
            // Room for the longest at once, so typing never moves it and
            // leaves a copy behind.
            text: String::with_capacity(passcode::MAX_LEN * 4),
            count: Animated::new(0.0, role::tick()).with_rest(0.004),
            shake: Animated::new(0.0, shake_spring()).with_rest(0.05),
            fault: Animated::new(0.0, role::effect()).with_rest(0.004),
            width: Animated::new(FIELD_W, role::release()).with_rest(0.2),
            placed: false,
        }
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Letters, digits and signs go in; Backspace takes one off; Esc
    /// clears; Enter submits.
    pub fn key(&mut self, key: Key) -> Edit {
        match key {
            Key::Char(c) if passcode::allowed(c) && self.len() < passcode::MAX_LEN => {
                self.text.push(c);
                self.count.set(self.len() as f32);
                Edit::Typed
            }
            Key::Backspace | Key::Delete if !self.text.is_empty() => {
                self.text.pop();
                self.count.set(self.len() as f32);
                Edit::Typed
            }
            Key::Back if !self.text.is_empty() => {
                self.clear();
                Edit::Typed
            }
            Key::Enter => Edit::Submit,
            _ => Edit::Nothing,
        }
    }

    /// What was typed, taken out of the field (which is left empty).
    /// What was typed, for the one who wipes it; the field's own copy is
    /// wiped now.
    pub fn take(&mut self) -> String {
        let text = self.text.clone();
        passcode::wipe(&mut self.text);
        self.count.set(0.0);
        text
    }

    pub fn clear(&mut self) {
        passcode::wipe(&mut self.text);
        self.count.set(0.0);
    }

    /// The passcode was wrong: the field shakes, its rim tints, and empties.
    pub fn wrong(&mut self, calm: bool) {
        self.clear();
        if calm {
            self.count.jump(0.0);
            self.fault.jump(1.0);
            self.fault.set(0.0);
            return;
        }
        self.shake.nudge(-SHAKE_KICK);
        self.fault.jump(1.0);
        self.fault.set(0.0);
    }

    pub fn tick(&mut self, dt: f32, calm: bool) -> bool {
        if calm {
            self.count.jump(self.count.target());
            self.width.jump(self.width.target());
        }
        self.count.tick(dt) | self.shake.tick(dt) | self.fault.tick(dt) | self.width.tick(dt)
    }

    /// Whether it is still moving (shaking, its dots growing, widening).
    pub fn moving(&self) -> bool {
        !self.count.is_settled() || !self.shake.is_settled() || !self.fault.is_settled() || !self.width.is_settled()
    }

    /// What the field says when no dots show: an icon, words, their colour.
    fn words(g: &Gfx, prompt: &Prompt) -> (&'static str, String, Color) {
        let t = g.tokens;
        match prompt {
            Prompt::Asks(words) => ("lock", words.clone(), t.dim),
            Prompt::Checking => ("lock", "checking".to_owned(), t.dim),
            Prompt::Wrong(words) => ("lock", words.clone(), g.fault()),
            Prompt::Wait(s) => ("timer", format!("try again in {}", passcode::wait_text(*s)), t.dim),
            Prompt::Alert(words) => ("battery_alert", words.clone(), t.amber_ink),
            Prompt::Open => ("lock_open", String::new(), t.ink),
            Prompt::Curtain => ("lock_open", "any key to go back".to_owned(), t.dim),
        }
    }

    /// The capsule centred on `cx`, its top at `y`, no wider than `most`:
    /// glass over what is drawn beneath it.
    pub fn draw(&mut self, g: &mut Gfx, cx: f32, y: f32, most: f32, prompt: &Prompt) {
        let t = g.tokens;
        let ty = g.ty;
        let shown = self.count.value();
        let dots = shown > 0.02 && !matches!(prompt, Prompt::Wait(_));
        let (icon, words, color) = Self::words(g, prompt);
        let want = if dots { FIELD_W } else { (g.measure(&words, ty.caption) + 56.0).max(FIELD_W) }.min(most);
        if (want - self.width.target()).abs() > 0.5 {
            let growing = want > self.width.target();
            self.width.set_with(want, if growing { role::release() } else { role::smooth() });
            g.started_moving();
        }
        if g.calm || !self.placed {
            self.width.jump(want);
            self.placed = true;
        }
        let w = self.width.value().min(most);
        let r = Rect::new((cx - w * 0.5).round() + self.shake.value().round(), y, w.round(), FIELD_H);
        g.glass(&[GlassShape::new(r, r.h * 0.5, 2)]);
        let fault = self.fault.value().clamp(0.0, 1.0);
        if fault > 0.01 {
            g.canvas.stroke_rrect(r, r.h * 0.5, 1.5, g.fault(), fault);
        }
        if dots {
            let n = (shown.ceil() as usize).max(self.len());
            let pitch = ((r.w - 28.0) / n.max(1) as f32).min(12.0);
            let x0 = r.cx() - pitch * (n as f32 - 1.0) * 0.5;
            let alpha = if *prompt == Prompt::Checking { 0.45 } else { 1.0 };
            for i in 0..n {
                let k = (shown - i as f32).clamp(0.0, 1.12);
                if k <= 0.01 {
                    continue;
                }
                g.canvas.circle(x0 + i as f32 * pitch, r.cy(), 3.4 * k, t.ink, alpha * k.min(1.0));
            }
            return;
        }
        let style = ty.caption;
        // Whole, or (where even the widest field is too narrow, at the
        // largest text) cut short; new words fade in as the field reaches
        // the width they need, as the island's do.
        let words = g.text.fit(&words, &style, (most - 44.0).max(0.0));
        // (An icon alone, the lock opening, sits in the middle.)
        let tw = if words.is_empty() { 12.0 } else { g.measure(&words, style) + 20.0 };
        let alpha = (((r.w - 24.0) / tw - 0.85) / 0.15).clamp(0.0, 1.0);
        let x = r.cx() - tw * 0.5;
        g.canvas.set_clip(r.inset(2.0));
        g.icon_alpha(icon, true, x + 6.0, r.cy(), 13.0, color, alpha);
        g.text.draw(&mut g.canvas, x + 18.0, (r.cy() + style.size * 0.36).round(), &words, &style, color, alpha);
        g.canvas.clear_clip();
    }
}

// ------------------------------------------------------------ the layout ---

/// The parts of the lock screen, for the editor to light up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Time,
    Date,
    Item(Show),
}

/// What the lock screen shows besides the time, gathered by the shell.
#[derive(Clone, Default)]
pub struct Info {
    pub h24: bool,
    pub reading: Option<Reading>,
    pub next: Option<crate::upnext::NextMatch>,
    pub reminder: Option<crate::upnext::Reminder>,
    /// Notifications not yet seen: how many, and the newest of them, newest
    /// first (the lock screen shows a few; their words only if chosen).
    pub notices: usize,
    pub notes: Vec<crate::notify::Notification>,
    /// The notification picked on the lock screen (↑ ↓): unlocked, it opens.
    pub pick: Option<u64>,
    /// Live activities, most important first (the lock screen shows two).
    pub activities: Vec<crate::liveactivity::Activity>,
    /// Music's level (1 playing, 0 paused, springing between).
    pub level: f32,
    /// Seconds since the lock screen began to come in, while its parts are
    /// still rising into place (none: all in place).
    pub rise: Option<f32>,
    /// The team's next match is near (depth.rs: the robot stands ready).
    pub soon: bool,
    /// The robot in front of the time as the lock screen has it now, on its
    /// springs (none: at rest in the pose this says).
    pub depth: Option<crate::depth::Shot>,
    /// The robot is left out, for the lock screen to lay over the rest.
    pub robot_apart: bool,
}

/// How far below its place a part of the lock screen still is, and how
/// present, `order` steps into its entrance (Bezel's launcher cascade: each
/// part 50 ms after the one before, rising 14 px on the release spring).
pub fn rise(info: &Info, order: f32) -> (f32, f32) {
    let Some(t) = info.rise else { return (0.0, 1.0) };
    let since = t - order * 0.05;
    if since <= 0.0 {
        return (RISE, 0.0);
    }
    (RISE * role::release().solve(1.0, 0.0, since).0, (since / 0.22).min(1.0))
}

const RISE: f32 = 14.0;
/// The entrance is over by then: the last part has settled.
pub const RISE_FOR: f32 = 0.9;

/// The lock screen's backdrop: the ground, with the colour's soft glow above
/// and to the left (where Bezel's light comes from) and a fainter one below.
pub fn backdrop(g: &mut Gfx, tint: Color) {
    let t = g.tokens;
    g.canvas.fill_rect(Rect::new(0.0, 0.0, W, H), t.ground, 1.0);
    let dark = t.tone == bezel::Tone::Dark;
    g.canvas.glow(56.0, 0.0, 200.0, tint, if dark { 0.16 } else { 0.22 });
    g.canvas.glow(W - 30.0, H + 10.0, 150.0, tint, if dark { 0.07 } else { 0.10 });
}

/// The chips that show with the time, in order: (part, icon, dot, words,
/// fewer words where those do not fit, switched on). The notices' words,
/// where they are to show, stand in for how many there are.
type Item = (Show, Option<&'static str>, Option<Color>, String, Option<String>, bool);

fn items(choices: &Choices, info: &Info, unix: i64, editing: bool) -> Vec<Item> {
    let mut out = Vec::new();
    for what in [Show::Charge, Show::Match, Show::Notices, Show::Reminder] {
        let on = choices.shows(what);
        if !on && !editing {
            continue;
        }
        let item = match what {
            Show::Charge => info.reading.as_ref().map(|r| {
                let percent = format!("{}%", r.percent);
                let charging = !r.flow.on_battery();
                (Some(if charging { "bolt" } else { "battery_full" }), None, if charging { format!("{percent} · charging") } else { percent.clone() }, Some(percent))
            }),
            Show::Match => info.next.as_ref().map(|m| (None, Some(m.color), format!("{} · {}", m.label, m.when), Some(m.label.clone()))),
            Show::Reminder if crate::upnext::reminders_available() => {
                info.reminder.as_ref().map(|r| (Some("checklist"), None, crate::upnext::reminder_line(r, unix), Some(r.title.clone())))
            }
            Show::Notices => {
                let count = match info.notices {
                    0 => "no notifications".to_owned(),
                    1 => "1 notification".to_owned(),
                    n => format!("{n} notifications"),
                };
                let newest = info.notes.first().map(|n| n.title.clone());
                match (info.notices, newest) {
                    (0, _) if !editing => None,
                    (1, Some(text)) if choices.previews => Some((Some("notifications"), None, text, Some(count))),
                    (n, Some(text)) if choices.previews && n > 1 => Some((Some("notifications"), None, format!("{text} · +{}", n - 1), Some(count))),
                    _ => Some((Some("notifications"), None, count, None)),
                }
            }
            _ => None,
        };
        if let Some((icon, dot, words, short)) = item {
            out.push((what, icon, dot, words, short, on));
        }
    }
    out
}

/// Lay the chips in rows `width` wide from `x` (centred on each row, from
/// its left, or to its right), the first row's middle at `cy`, as many rows
/// as fit above `bottom`: each whole on its row (a long one, the notices'
/// words, cut to the row), in order, as many as fit, rising in from `order`.
/// Records where each went.
#[allow(clippy::too_many_arguments)]
fn flow(g: &mut Gfx, chips: &[Item], x: f32, width: f32, cy: f32, align: Align, bottom: f32, info: &Info, order: f32, places: &mut Vec<(Part, Rect)>) {
    let t = g.tokens;
    let (gap, pitch) = (6.0, (g.ty.caption.size + 9.0).round() + 5.0);
    let rows = ((bottom - (cy - (pitch - 5.0) * 0.5) + 5.0) / pitch).floor().max(1.0) as usize;
    // Rows of (chip, its words, its width), if every one of `keep` fits.
    let arrange = |g: &mut Gfx, keep: &[usize], short: bool| -> Option<Vec<Vec<(usize, String, f32)>>> {
        let mut lines: Vec<Vec<(usize, String, f32)>> = vec![Vec::new()];
        let mut used = 0.0;
        for &i in keep {
            let (_, icon, dot, words, fewer, _) = &chips[i];
            let lead = icon.is_some() || dot.is_some();
            // The whole words, unless they are too long for a row (or
            // fewer are asked for, to make room).
            let words = match fewer {
                Some(fewer) if short || chip_width(g, lead, words) > width => fewer,
                _ => words,
            };
            let words = g.text.fit(words, &g.ty.caption, width - (if lead { 26.0 } else { 18.0 }));
            let w = chip_width(g, lead, &words);
            let first = lines.last().is_some_and(Vec::is_empty);
            if !first && used + gap + w > width {
                if lines.len() == rows {
                    return None;
                }
                lines.push(Vec::new());
                used = w;
            } else {
                used = if first { w } else { used + gap + w };
            }
            lines.last_mut().expect("a row").push((i, words, w));
        }
        Some(lines)
    };
    // Every chip, in whole words or fewer; else the least needed goes first
    // (the reminder, then the charge, then the match): notices go last.
    let rank = |s: Show| match s {
        Show::Reminder => 0,
        Show::Charge => 1,
        Show::Match => 2,
        _ => 3,
    };
    let mut keep: Vec<usize> = (0..chips.len()).collect();
    let lines = loop {
        if let Some(lines) = arrange(g, &keep, false).or_else(|| arrange(g, &keep, true)) {
            break lines;
        }
        let Some(least) = keep.iter().copied().min_by_key(|i| rank(chips[*i].0)) else { break Vec::new() };
        keep.retain(|i| *i != least);
    };
    for (row, line) in lines.iter().enumerate() {
        let total: f32 = line.iter().map(|(_, _, w)| w).sum::<f32>() + gap * line.len().saturating_sub(1) as f32;
        let mut at = match align {
            Align::Center => x + (width - total) * 0.5,
            Align::Left => x,
            Align::Right => x + width - total,
        };
        let (dy, alpha) = rise(info, order + row as f32);
        let y = cy + row as f32 * pitch;
        for (i, words, w) in line {
            let (what, icon, dot, _, _, on) = &chips[*i];
            let shown = if *on { 1.0 } else { 0.3 };
            chip(g, at.round(), (y + dy).round(), *icon, *dot, words, t.surface2, t.ink, shown * alpha);
            places.push((Part::Item(*what), Rect::new(at.round(), y - pitch * 0.5 + 2.5, *w, pitch - 5.0)));
            at += w + gap;
        }
    }
}

/// Where the lock screen put its parts, and where its field goes (its
/// middle, and how wide it may grow).
pub struct Layout {
    pub places: Vec<(Part, Rect)>,
    pub field: (f32, f32),
    /// The notifications shown, by id, and where.
    pub rows: Vec<(u64, Rect)>,
    /// Everything drawn, by what it is, as far as it reaches (for the test
    /// that no two meet).
    #[cfg_attr(not(test), allow(dead_code))]
    pub parts: Vec<(String, Rect)>,
}

impl Layout {
    /// The time and the date as drawn (`ink`), and the chips where they went.
    fn of(places: Vec<(Part, Rect)>, field: (f32, f32), mut ink: Vec<(String, Rect)>) -> Layout {
        ink.extend(places.iter().filter(|(p, _)| matches!(p, Part::Item(_))).map(|(_, r)| ("chip".to_owned(), *r)));
        Layout { places, field, rows: Vec::new(), parts: ink }
    }
}

/// Draw the lock screen's face (all but the field) as `choices` have it.
/// `editing`: the editor's preview, where what is switched off still shows,
/// faintly. Each part rises into place in turn as the lock screen comes in.
///
/// - **Centre**: the date, the time large in the middle, the chips beneath
///   in up to two rows, the field low and centred.
/// - **Dots**: the same, the time set in dots.
/// - **Stacked**: the hours over the minutes, large, down the left; the
///   date, the chips and the field down the right.
/// - **With live activities** (music playing, a timer, the queue) or
///   notifications not yet seen, any of them: the time smaller along the
///   top (in its style and colour) with the date and the chips beside it,
///   and beneath, the activities' cards (the most important first: one
///   large, or two) and the notifications, as many as fit.
pub fn draw_face(g: &mut Gfx, choices: &Choices, info: &Info, now: NaiveDateTime, editing: bool) -> Layout {
    let t = g.tokens;
    let ty = g.ty;
    let tint = choices.colour.color(&t);
    backdrop(g, tint);
    let unix = chrono::Utc::now().timestamp();
    let mut places = Vec::new();
    let time = time_text(now, info.h24);
    let date_on = choices.shows(Show::Date);
    let date_alpha = if date_on { 1.0 } else { 0.3 };
    let mut chips = items(choices, info, unix, editing);
    let notes = choices.shows(Show::Notices) && !info.notes.is_empty();
    // A match coming is the robot's to show, standing ready (depth.rs): its
    // activity alone keeps the time large with the robot in front of it.
    let matches_only = info.activities.iter().all(|a| a.kind == crate::liveactivity::Kind::Match);
    let depth = choices.depth_on() && !notes && matches_only;
    if (!info.activities.is_empty() || notes) && !depth {
        // What an activity already shows is not a chip as well.
        use crate::liveactivity::Kind;
        chips.retain(|c| !info.activities.iter().any(|a| (c.0 == Show::Match && a.kind == Kind::Match) || (c.0 == Show::Reminder && a.kind == Kind::Reminder)));
        return band(g, choices, info, now, &time, &chips, editing, tint, date_alpha);
    }
    if depth {
        return depth_face(g, choices, info, now, &time, chips, editing, tint, date_alpha);
    }
    let mut ink: Vec<(String, Rect)> = Vec::new();
    match choices.lock_face {
        LockFace::Centre | LockFace::Dots => {
            let date_cy = 15.0;
            if date_on || editing {
                let date = date_text(now);
                let date = if g.measure(&date, ty.caption) > W - 2.0 * PAD { short_date(now) } else { date };
                let w = g.measure(&date, ty.caption);
                let (dy, alpha) = rise(info, 0.0);
                g.text.draw_aligned(&mut g.canvas, W * 0.5, (date_cy + dy + ty.caption.size * 0.36).round(), Align::Center, &date, &ty.caption, t.dim, date_alpha * alpha);
                places.push((Part::Date, Rect::new((W - w) * 0.5 - 6.0, date_cy - 8.0, w + 12.0, 16.0)));
                ink.push(("date".into(), Rect::new((W - w) * 0.5, date_cy - ty.caption.size * 0.5, w, ty.caption.size)));
            }
            let (dy, alpha) = rise(info, 1.0);
            let below = if choices.lock_face == LockFace::Dots {
                let (pitch, top) = (6.5, 32.0);
                let (cols, _) = crate::faces::dot_time(&time);
                draw_dots(g, &time, &time, 1.0, W * 0.5, top + dy, pitch, tint, alpha);
                let w = cols as f32 * pitch;
                places.push((Part::Time, Rect::new((W - w) * 0.5 - 4.0, top - 6.0, w + 8.0, 6.0 * pitch + 12.0)));
                ink.push(("time".into(), Rect::new((W - w) * 0.5, top - pitch * 0.5, w, 7.0 * pitch)));
                top + 6.0 * pitch + 19.0
            } else {
                let style = ty.display(54.0);
                let tab = Tab::new(g, style);
                let cap = g.text.cap_height(&style);
                let w = tab.width(&time);
                let baseline = (27.0 + cap).round();
                tab.draw(g, &time, &time, 1.0, (W - w) * 0.5, baseline + dy, tint, alpha);
                places.push((Part::Time, Rect::new((W - w) * 0.5 - 6.0, baseline - cap - 3.0, w + 12.0, cap + 6.0)));
                ink.push(("time".into(), Rect::new((W - w) * 0.5, baseline - cap, w, cap)));
                baseline + 19.0
            };
            flow(g, &chips, PAD, W - 2.0 * PAD, below, Align::Center, FIELD_Y - 4.0, info, 2.0, &mut places);
            Layout::of(places, (W * 0.5, W - 2.0 * PAD - 12.0), ink)
        }
        LockFace::Stacked => {
            // The hours over the minutes, down the left.
            let (hours, minutes) = time.split_once(':').unwrap_or((time.as_str(), ""));
            let style = ty.display(62.0);
            let tab = Tab::new(g, style);
            let cap = g.text.cap_height(&style);
            let gap = 12.0;
            let top = ((H - (2.0 * cap + gap)) * 0.5).round();
            let x = PAD + 6.0;
            let first = top + cap;
            let second = first + gap + cap;
            let (dy, alpha) = rise(info, 0.0);
            tab.draw(g, hours, hours, 1.0, x, first + dy, tint, alpha);
            let (dy, alpha) = rise(info, 1.0);
            tab.draw(g, minutes, minutes, 1.0, x, second + dy, tint, alpha);
            let w = tab.width("88");
            places.push((Part::Time, Rect::new(x - 6.0, top - 4.0, w + 12.0, 2.0 * cap + gap + 8.0)));
            ink.push(("time".into(), Rect::new(x, top, w, 2.0 * cap + gap)));
            // The rest down the right.
            let col = x + w + 16.0;
            let width = W - PAD - 2.0 - col;
            if date_on || editing {
                let date = date_text(now);
                let date = if g.measure(&date, ty.caption) > width { short_date(now) } else { date };
                let (dy, alpha) = rise(info, 1.0);
                let dw = g.text.draw(&mut g.canvas, col, (top + 6.0 + dy + ty.caption.size * 0.36).round(), &date, &ty.caption, t.dim, date_alpha * alpha);
                places.push((Part::Date, Rect::new(col - 6.0, top - 2.0, dw + 12.0, 16.0)));
                ink.push(("date".into(), Rect::new(col, top + 6.0 - ty.caption.size * 0.5, dw, ty.caption.size)));
            }
            flow(g, &chips, col, width, top + 30.0, Align::Left, FIELD_Y - 4.0, info, 2.0, &mut places);
            Layout::of(places, (col + width * 0.5, width), ink)
        }
    }
}

/// How large the robot stands on the lock screen: small enough that the
/// time behind it can be large, its feet just above the field. (Our own
/// tuning.)
pub const DEPTH_SCALE: f32 = 0.78;
const DEPTH_GROUND: f32 = FIELD_Y - 5.0;

/// The bumpers in the alliance's colour when it stands ready for a match
/// the team is in.
pub fn ready_paint(pose: crate::depth::Pose, info: &Info) -> Option<Color> {
    (pose == crate::depth::Pose::Ready).then(|| info.next.as_ref().filter(|m| m.side.is_some()).map(|m| m.color)).flatten()
}

/// The match is near: the schedule says so, or its live activity shows.
fn match_near(info: &Info) -> bool {
    info.soon || info.activities.iter().any(|a| a.kind == crate::liveactivity::Kind::Match)
}

/// The robot's pose for what the lock screen shows.
pub fn depth_pose(choices: &Choices, info: &Info) -> crate::depth::Pose {
    crate::depth::choose(choices.follows, info.reading.as_ref(), match_near(info))
}

/// The lock screen with the robot in front of the time (depth.rs): along
/// the top, the date at the left and the chips at the right; the time
/// large beneath them; the robot standing just above the field, in front
/// of the lower part of the digits (`depth::COVER` of them, never more than
/// a third). Drawn back to front: the charge light, the time, the robot,
/// then the date and the chips over everything. Standing ready, the match
/// it is ready for shows among the chips, chosen or not.
#[allow(clippy::too_many_arguments)]
fn depth_face(g: &mut Gfx, choices: &Choices, info: &Info, now: NaiveDateTime, time: &str, mut chips: Vec<Item>, editing: bool, tint: Color, date_alpha: f32) -> Layout {
    use crate::depth::{self, Pose};
    let t = g.tokens;
    let ty = g.ty;
    let mut places = Vec::new();
    let mut ink: Vec<(String, Rect)> = Vec::new();
    let shot = info.depth.unwrap_or_else(|| {
        let pose = depth_pose(choices, info);
        depth::Shot::settled(pose, ready_paint(pose, info))
    });
    // The match, as its live activity says it ("Q31 · queue 8:00") where
    // there is one, else as the schedule does; standing ready, chosen or not.
    let activity = info.activities.iter().find(|a| a.kind == crate::liveactivity::Kind::Match);
    let colour = info.next.as_ref().map(|m| m.color).or(activity.and_then(|a| match &a.mark {
        crate::liveactivity::Mark::Icon(_, crate::liveactivity::Tint::Fixed(c)) => Some(*c),
        _ => None,
    }));
    let words = activity.map(|a| (a.short.clone(), a.title.clone())).or(info.next.as_ref().map(|m| (format!("{} · {}", m.label, m.when), m.label.clone())));
    let shown = choices.shows(Show::Match) || shot.pose == Pose::Ready;
    chips.retain(|c| c.0 != Show::Match);
    if let Some((words, label)) = words.filter(|_| shown || editing) {
        let at = chips.iter().position(|c| c.0 != Show::Charge).unwrap_or(chips.len());
        chips.insert(at, (Show::Match, None, Some(colour.unwrap_or(t.dim)), words, Some(label), shown));
    }
    // The row along the top, and beneath it the time and the robot.
    let chip_h = (ty.caption.size + 9.0).round();
    let row_cy = (5.0 + chip_h * 0.5).round();
    let from = row_cy + chip_h * 0.5 + 7.0;
    let (scale, ground, cx) = (DEPTH_SCALE, DEPTH_GROUND, W * 0.5);
    let room = W - 2.0 * PAD - 8.0;
    let probe = if info.h24 { "88:88" } else { "18:88" };
    let (dy, alpha) = rise(info, 1.5);
    depth::draw_light(g, &shot, cx, ground + dy, scale, alpha);
    let (dy, alpha) = rise(info, 1.0);
    let time_ink = if choices.lock_face == LockFace::Dots {
        // As many dots high as the cap would be: seven rows, as wide as fits.
        let cols = crate::faces::dot_time(probe).0 as f32;
        let (want, _) = depth::time_for(from, ground, scale, f32::MAX);
        let pitch = (want / 7.0).min(room / cols).floor();
        let (cap, baseline) = depth::time_for(from, ground, scale, 7.0 * pitch);
        let top = baseline - cap + pitch * 0.5;
        draw_dots(g, time, time, 1.0, cx, top + dy, pitch, tint, alpha);
        let w = crate::faces::dot_time(time).0 as f32 * pitch;
        Rect::new(cx - w * 0.5, baseline - cap, w, cap)
    } else {
        let wide = Tab::sized_to(g, ty.display(100.0), probe, room, 200.0);
        let k = g.text.cap_height(&wide) / wide.size;
        let (cap, _) = depth::time_for(from, ground, scale, g.text.cap_height(&wide));
        let style = bezel::TextStyle { size: (cap / k).floor(), ..wide };
        // Its own cap, so the robot covers no more than it should.
        let (cap, baseline) = depth::time_for(from, ground, scale, g.text.cap_height(&style));
        let tab = Tab::new(g, style);
        let w = tab.width(time);
        let x = ((W - w) * 0.5).round();
        tab.draw(g, time, time, 1.0, x, baseline + dy, tint, alpha);
        Rect::new(x, baseline - cap, w, cap)
    };
    places.push((Part::Time, time_ink.inset(-4.0)));
    ink.push(("time".into(), time_ink));
    // (Laid over the rest afterwards by the lock screen, when it keeps the rest.)
    if !info.robot_apart {
        let (dy, alpha) = rise(info, 1.5);
        depth_robot(g, &shot, dy, alpha);
    }
    ink.push(("robot".into(), depth::bounds(cx, ground + shot.sink.max(0.0), scale)));
    // Over everything: the date at the left, the chips at the right (the
    // whole date where the chips leave it room).
    let x = PAD + 2.0;
    let right = W - PAD;
    let mut left = x;
    if choices.shows(Show::Date) || editing {
        let want: f32 = chips.iter().map(|c| chip_width(g, c.1.is_some() || c.2.is_some(), &c.3)).sum::<f32>() + 6.0 * chips.len().saturating_sub(1) as f32;
        let long = date_text(now);
        let date = if g.measure(&long, ty.caption) <= right - x - want - 12.0 { long } else { short_date(now) };
        let w = g.measure(&date, ty.caption);
        let (dy, alpha) = rise(info, 0.0);
        g.text.draw(&mut g.canvas, x, (row_cy + dy + ty.caption.size * 0.36).round(), &date, &ty.caption, t.dim, date_alpha * alpha);
        places.push((Part::Date, Rect::new(x - 6.0, row_cy - 8.0, w + 12.0, 16.0)));
        ink.push(("date".into(), Rect::new(x, row_cy - ty.caption.size * 0.5, w, ty.caption.size)));
        left = x + w + 12.0;
    }
    flow(g, &chips, left, right - left, row_cy, Align::Right, row_cy + chip_h * 0.5, info, 2.0, &mut places);
    Layout::of(places, (W * 0.5, W - 2.0 * PAD - 12.0), ink)
}

/// The lock screen with live activities or notifications: the time along
/// the top in its style and colour; at the right of it the date, and under
/// the date one row of chips; and beneath them all (below whichever of the
/// time and the chips reaches lower, so nothing ever runs under another),
/// the activities' cards and the notifications.
///
/// - Two activities or more: their two cards (a third and more said in a
///   chip), the notifications counted in a chip.
/// - One: its card, and under it the newest notification if the card keeps
///   room enough (else the card alone, the notifications in a chip).
/// - None: the notifications, as many as fit, the last saying how many more.
#[allow(clippy::too_many_arguments)]
fn band(g: &mut Gfx, choices: &Choices, info: &Info, now: NaiveDateTime, time: &str, chips: &[Item], editing: bool, tint: Color, date_alpha: f32) -> Layout {
    let t = g.tokens;
    let ty = g.ty;
    let mut places = Vec::new();
    let mut parts: Vec<(String, Rect)> = Vec::new();
    let x = PAD + 6.0;
    let top = 8.0;
    let (dy, alpha) = rise(info, 0.0);
    let (time_w, time_h) = if choices.lock_face == LockFace::Dots {
        let pitch = 4.2;
        let (cols, _) = crate::faces::dot_time(time);
        let w = cols as f32 * pitch;
        draw_dots(g, time, time, 1.0, x + (w - pitch) * 0.5, top + 3.0 + dy, pitch, tint, alpha);
        (w, 6.0 * pitch + 6.0)
    } else {
        let style = ty.display(38.0);
        let tab = Tab::new(g, style);
        let cap = g.text.cap_height(&style);
        tab.draw(g, time, time, 1.0, x - 1.0, (top + cap + dy).round(), tint, alpha);
        (tab.width(time), cap)
    };
    places.push((Part::Time, Rect::new(x - 6.0, top - 4.0, time_w + 12.0, time_h + 8.0)));
    parts.push(("time".into(), Rect::new(x - 1.0, top, time_w, time_h)));
    let mut head = top + time_h;
    // The date at the right of the time, and a row of chips under it.
    let right = W - PAD - 2.0;
    let room = right - (x + time_w + 14.0);
    let date_cy = top + 6.0;
    let mut date_bottom = top;
    if choices.shows(Show::Date) || editing {
        let date = date_text(now);
        let date = if g.measure(&date, ty.caption) > room { short_date(now) } else { date };
        let (dy, alpha) = rise(info, 1.0);
        let dw = g.measure(&date, ty.caption);
        g.text.draw_aligned(&mut g.canvas, right, (date_cy + dy + ty.caption.size * 0.36).round(), Align::Right, &date, &ty.caption, t.dim, date_alpha * alpha);
        places.push((Part::Date, Rect::new(right - dw - 6.0, top - 2.0, dw + 12.0, 16.0)));
        let r = Rect::new(right - dw, date_cy - ty.caption.size * 0.5, dw, ty.caption.size);
        parts.push(("date".into(), r));
        date_bottom = r.bottom();
        head = head.max(r.bottom());
    }
    let chip_h = (ty.caption.size + 9.0).round();
    let chips_cy = (date_bottom + 4.0 + chip_h * 0.5).round();
    // What goes beneath, and whether the notifications get rows of their
    // own there (the room is worked out as if the chips' row were there).
    let acts = &info.activities[..info.activities.len().min(2)];
    let notes: &[crate::notify::Notification] = if choices.shows(Show::Notices) { &info.notes } else { &[] };
    let bottom = FIELD_Y - 6.0;
    let gap = 5.0;
    let content_top = (head.max(chips_cy + chip_h * 0.5) + 6.0).round();
    let avail = bottom - content_top;
    let (card_min, one_h, two_h) = (ty.body.size + ty.caption.size + 11.0, note_h(g, false), note_h(g, true));
    let note_row = match acts.len() {
        _ if notes.is_empty() => None,
        0 => Some(0.0),
        // Under one card: two lines if the card keeps its room, else one.
        1 if choices.previews && avail - gap - two_h >= card_min => Some(two_h),
        1 if avail - gap - one_h >= card_min => Some(one_h),
        _ => None,
    };
    let more = info.activities.len().saturating_sub(2);
    let mut chips: Vec<Item> = chips.iter().filter(|c| !(note_row.is_some() && c.0 == Show::Notices)).cloned().collect();
    if more > 0 {
        chips.push((Show::Notices, Some("graphic_eq"), None, format!("+{more}"), None, true));
    }
    let before = places.len();
    flow(g, &chips, right - room, room, chips_cy, Align::Right, chips_cy + chip_h * 0.5, info, 2.0, &mut places);
    for (_, r) in &places[before..] {
        parts.push(("chip".into(), *r));
        head = head.max(r.bottom());
    }
    // Beneath: from below the time and the chips down to above the field.
    let content_top = (head + 6.0).round();
    let avail = bottom - content_top;
    let mut rows = Vec::new();
    let card = |g: &mut Gfx, parts: &mut Vec<(String, Rect)>, a: &crate::liveactivity::Activity, y: f32, h: f32, order: f32| {
        let (dy, alpha) = rise(info, order);
        let r = Rect::new(PAD, (y + dy).round(), W - 2.0 * PAD, h.floor());
        g.canvas.rrect(r, g.radius(14.0).min(r.h * 0.5), t.surface1, alpha);
        crate::liveactivity::draw_card(g, r, a, info.level, alpha);
        parts.push(("card".into(), Rect::new(PAD, y.round(), W - 2.0 * PAD, h.floor())));
    };
    match (acts.len(), note_row) {
        (0, _) => {
            // As many as fit, each as tall as its words need, stacked up
            // from just above the field (as a phone's do): two lines where
            // they show and there is room, else one each.
            let want = notes.len().min(3);
            let fit = |h: f32| (((avail + gap) / (h + gap)).floor().max(1.0) as usize).min(want);
            let two = choices.previews && fit(two_h) >= fit(one_h);
            let h = if two { two_h } else { one_h };
            let n = fit(h);
            let first = (bottom - n as f32 * h - (n as f32 - 1.0) * gap).max(content_top);
            for (i, n_) in notes.iter().take(n).enumerate() {
                let y = first + i as f32 * (h + gap);
                let left = if i + 1 == n { notes.len() - n } else { 0 };
                let r = note(g, info, n_, y, h, two, choices.previews, left, 3.0 + i as f32);
                parts.push(("note".into(), r));
                rows.push((n_.id, r));
            }
        }
        (1, Some(h)) => {
            card(g, &mut parts, &acts[0], content_top, avail - gap - h, 3.0);
            let two = h > one_h;
            let r = note(g, info, &notes[0], bottom - h, h, two, choices.previews, notes.len() - 1, 4.0);
            parts.push(("note".into(), r));
            rows.push((notes[0].id, r));
        }
        (n, _) => {
            let h = (avail - gap * (n as f32 - 1.0)) / n as f32;
            for (i, a) in acts.iter().enumerate() {
                card(g, &mut parts, a, content_top + i as f32 * (h + gap), h, 3.0 + i as f32);
            }
        }
    }
    Layout { places, field: (W * 0.5, W - 2.0 * PAD - 12.0), rows, parts }
}

/// A notification's row on the lock screen: two lines (its title, and the
/// start of what it says) or one.
fn note_h(g: &Gfx, two: bool) -> f32 {
    let ty = g.ty;
    if two {
        ty.body.size + ty.caption.size + 13.0
    } else {
        ty.body.size + 12.0
    }
}

/// A notification on the lock screen, its top at `y`: its app's symbol;
/// its title (and on a second line the start of what it says) where the
/// words show, else its app's name alone; when, at the right, or on the
/// last row how many more there are. Picked (↑ ↓), it has the focus ring.
/// Where it went, as at rest.
#[allow(clippy::too_many_arguments)]
fn note(g: &mut Gfx, info: &Info, n: &crate::notify::Notification, y: f32, h: f32, two: bool, words: bool, more: usize, order: f32) -> Rect {
    let t = g.tokens;
    let ty = g.ty;
    let (dy, alpha) = rise(info, order);
    let at = Rect::new(PAD, y.round(), W - 2.0 * PAD, h.floor());
    let r = Rect::new(at.x, at.y + dy.round(), at.w, at.h);
    let radius = g.radius(12.0).min(r.h * 0.5);
    g.canvas.rrect(r, radius, t.surface1, alpha);
    let x = r.x + 30.0;
    let side = if more > 0 { format!("+{more}") } else { crate::notify::ago(n.at, chrono::Utc::now().timestamp()) };
    let side_w = g.measure(&side, ty.caption);
    let title = if words { n.title.as_str() } else { n.app.as_str() };
    let line2 = two && words && !n.body.is_empty();
    let (first_cy, second_cy) = if line2 {
        let block = ty.body.size + 3.0 + ty.caption.size;
        let top = r.cy() - block * 0.5;
        (top + ty.body.size * 0.5, top + ty.body.size + 3.0 + ty.caption.size * 0.5)
    } else {
        (r.cy(), r.cy())
    };
    g.icon_alpha(&n.icon, true, r.x + 15.0, first_cy, 14.0, t.ink, alpha);
    g.text.draw_aligned(&mut g.canvas, r.right() - 10.0, (first_cy + ty.caption.size * 0.36).round(), Align::Right, &side, &ty.caption, t.dim, alpha);
    let room = r.right() - 10.0 - side_w - 8.0 - x;
    let title = g.text.fit(title, &ty.body, room);
    g.text.draw(&mut g.canvas, x, (first_cy + ty.body.size * 0.36).round(), &title, &ty.body, t.ink, alpha);
    if line2 {
        let body = g.text.fit(n.body.lines().next().unwrap_or(""), &ty.caption, r.right() - 10.0 - x);
        g.text.draw(&mut g.canvas, x, (second_cy + ty.caption.size * 0.36).round(), &body, &ty.caption, t.dim, alpha);
    }
    if info.pick == Some(n.id) {
        g.canvas.stroke_rrect(r.inset(-2.5), radius + 2.5, 1.5, g.ring_color(), alpha);
    }
    at
}

// ------------------------------------------------------------- the layer ---

/// The lock, over everything while it is engaged.
pub struct Lock {
    /// Locked: only the passcode opens it.
    engaged: bool,
    /// Shown by hand (Tab held) with no passcode set: the lock screen as a
    /// curtain, which any key lifts.
    curtain: bool,
    /// Seconds since the lock screen began to come in (its parts rising in
    /// turn), or none once they have all settled.
    entering: Option<f32>,
    /// The curtain was lifted by a key, to tell at the next tick.
    lifted: bool,
    /// A passcode is set (as last read), and when that was read.
    armed: bool,
    armed_generation: u64,
    armed_age: f32,
    /// How present the lock screen is: 1 covering, gliding to 0 as it opens.
    shown: Animated,
    pub field: Field,
    checking: Option<std::sync::mpsc::Receiver<bool>>,
    tries: Tries,
    wrong_words: Option<String>,
    /// The notifications shown last frame, by id, top first; the one picked
    /// among them (↑ ↓); and, once unlocked with one picked, it (for the
    /// shell to open).
    rows: Vec<u64>,
    pick: Option<u64>,
    opened: Option<u64>,
    /// A warning that outranks the lock (the battery running out).
    alert: Option<String>,
    /// The second last drawn of a wait, and the minute of the clock.
    drawn: (u64, String),
    /// The robot in front of the time, and whether it is held still (the
    /// battery saver, StandBy over it, the calm version).
    robot: crate::depth::Robot,
    still: bool,
    /// The picture under the robot, kept while it shows the same.
    kept: Option<Kept>,
}

/// The lock screen under its robot, kept: the backdrop, the time, the date
/// and the chips as `scene` has them; where the field goes; and the field's
/// strip, as `field` (its words, dots typed and width) had it.
struct Kept {
    scene: Scene,
    under: Vec<u32>,
    place: (f32, f32),
    field: Option<((Prompt, usize, i32), Vec<u32>)>,
}

/// The rows the field takes (and a row either side of it): the robot stops above them.
fn field_strip() -> std::ops::Range<usize> {
    let w = W as usize;
    (FIELD_Y as usize - 1) * w..((FIELD_Y + FIELD_H) as usize + 3).min(H as usize) * w
}

/// What the picture under the lock screen's robot shows, as far as it
/// changes it: while it is the same, the picture is kept.
#[derive(PartialEq)]
struct Scene {
    choices: Choices,
    time: String,
    day: chrono::NaiveDate,
    h24: bool,
    reading: Option<Reading>,
    next: Option<crate::upnext::NextMatch>,
    reminder: Option<crate::upnext::Reminder>,
    notices: usize,
    acts: usize,
    counting: Option<String>,
    soon: bool,
    pose: crate::depth::Pose,
    glow: i32,
    colours: [(u8, u8, u8); 6],
    caption: i32,
    calm: bool,
}

impl Scene {
    fn of(g: &Gfx, choices: &Choices, info: &Info, now: NaiveDateTime, shot: &crate::depth::Shot) -> Scene {
        let t = g.tokens;
        Scene {
            choices: *choices,
            time: time_text(now, info.h24),
            day: now.date(),
            h24: info.h24,
            reading: info.reading.clone(),
            next: info.next.clone(),
            reminder: info.reminder.clone(),
            notices: info.notes.len().max(info.notices),
            acts: info.activities.len(),
            // The match's activity counting down to its queue.
            counting: info.activities.iter().find(|a| a.kind == crate::liveactivity::Kind::Match).map(|a| a.short.clone()),
            soon: match_near(info),
            pose: shot.pose,
            glow: (shot.glow * 1000.0).round() as i32,
            colours: [t.ground, t.ink, t.dim, t.signal, t.surface2, t.ice].map(|c| (c.0, c.1, c.2)),
            caption: (g.ty.caption.size * 10.0) as i32,
            calm: g.calm,
        }
    }
}

/// The lock screen's robot: centred, its feet just above the field, cut
/// off above it (its shadow does not reach under the glass).
fn depth_robot(g: &mut Gfx, shot: &crate::depth::Shot, dy: f32, alpha: f32) {
    g.canvas.set_clip(Rect::new(0.0, 0.0, W, FIELD_Y - 1.0));
    crate::depth::draw(g, shot, W * 0.5, DEPTH_GROUND + dy, DEPTH_SCALE, alpha);
    g.canvas.clear_clip();
}

impl Default for Lock {
    fn default() -> Self {
        Self::new()
    }
}

impl Lock {
    /// Locked from the start when a passcode is set: a restart is no way in.
    pub fn new() -> Lock {
        let armed = passcode::is_set();
        Lock {
            engaged: armed,
            curtain: false,
            entering: armed.then_some(0.0),
            lifted: false,
            armed,
            armed_generation: passcode::generation(),
            armed_age: 0.0,
            shown: Animated::new(if armed { 1.0 } else { 0.0 }, role::release()).with_rest(0.004),
            field: Field::new(),
            checking: None,
            tries: Tries::load(passcode::unix_now()),
            wrong_words: None,
            rows: Vec::new(),
            pick: None,
            opened: None,
            alert: None,
            drawn: (0, String::new()),
            robot: crate::depth::Robot::new(),
            still: false,
            kept: None,
        }
    }

    /// The robot is to stand still (the battery saver, or StandBy over the
    /// lock screen): it neither turns by itself nor eases between poses.
    pub fn hold_still(&mut self, still: bool) {
        self.still = still;
    }

    /// The robot is turning on its stand: the shell draws a few frames a
    /// second for it (`depth::DRIFT_EVERY`), ambient, not motion.
    pub fn drifting(&self) -> bool {
        self.covering() && self.robot.drifts(self.still)
    }

    /// A passcode is set.
    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn engaged(&self) -> bool {
        self.engaged
    }

    /// The lock screen takes the keys: locked, or shown as a curtain.
    pub fn covering(&self) -> bool {
        self.engaged || self.curtain
    }

    /// Anything of the lock screen is on the panel.
    pub fn visible(&self) -> bool {
        self.covering() || self.shown.value() > 0.004
    }

    /// It covers the panel whole: nothing beneath need be drawn.
    pub fn covers(&self) -> bool {
        self.covering() && self.shown.value() >= 0.999
    }

    /// Locked by hand (Tab held): the lock screen comes down over the screen
    /// in front, its parts rising into place. With a passcode it is locked;
    /// without one it is a curtain any key lifts.
    pub fn show(&mut self, calm: bool) {
        if self.covering() {
            return;
        }
        if self.armed {
            self.engaged = true;
            self.field.clear();
            self.wrong_words = None;
            self.tries = Tries::load(passcode::unix_now());
        } else {
            self.curtain = true;
        }
        if calm {
            self.shown.jump(1.0);
        } else {
            self.shown.set_with(1.0, role::release());
        }
        self.entering = (!calm).then_some(0.0);
    }

    /// The screen woke onto the lock screen: its parts rise in again.
    pub fn woke(&mut self, calm: bool) {
        if self.covering() && !calm {
            self.entering = Some(0.0);
        }
    }

    /// Seconds into the entrance, while its parts are rising.
    pub fn entering(&self) -> Option<f32> {
        self.entering
    }

    /// Whether a passcode is set, read again when it changes (or now and
    /// then, in case it was taken off another way, over SSH).
    fn refresh(&mut self, dt: f32) {
        self.armed_age += dt;
        if passcode::generation() != self.armed_generation || self.armed_age >= 10.0 {
            self.armed_generation = passcode::generation();
            self.armed_age = 0.0;
            if let Some(set) = passcode::state() {
                self.armed = set;
            }
        }
    }

    /// The screen went off (or StandBy came): locked, if a passcode is set.
    pub fn engage(&mut self) {
        if !self.armed || self.engaged {
            return;
        }
        self.engaged = true;
        self.curtain = false;
        self.shown.jump(1.0);
        self.field.clear();
        self.wrong_words = None;
        self.tries = Tries::load(passcode::unix_now());
    }

    /// The notification picked on the lock screen, if any.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn pick(&self) -> Option<u64> {
        self.pick
    }

    /// Unlocked with a notification picked: it, once (to be opened).
    pub fn take_opened(&mut self) -> Option<u64> {
        self.opened.take()
    }

    /// ↑ ↓ among the notifications shown: from none to the first (↓) or the
    /// last (↑), and on to none again past either end.
    fn step_pick(&mut self, down: bool) {
        let at = self.pick.and_then(|id| self.rows.iter().position(|r| *r == id));
        let next = match (at, down) {
            (None, true) => Some(0),
            (None, false) => self.rows.len().checked_sub(1),
            (Some(i), true) => (i + 1 < self.rows.len()).then_some(i + 1),
            (Some(i), false) => i.checked_sub(1),
        };
        self.pick = next.map(|i| self.rows[i]);
    }

    /// A warning shown over the lock (the battery running out).
    pub fn alert(&mut self, text: impl Into<String>) {
        self.alert = Some(text.into());
    }

    /// Seconds before the next try is taken (after too many wrong).
    pub fn wait(&self) -> u64 {
        self.tries.wait(passcode::unix_now())
    }

    /// A passcode typed is being checked (for the galleries and the tests).
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn checking(&self) -> bool {
        self.checking.is_some()
    }

    /// A key on the lock screen. ↑ and ↓ pick a notification shown on it
    /// (unlocked, it opens). Shown as a curtain (no passcode), Enter lifts
    /// it (and opens the one picked) and any other key lifts it, spent
    /// doing so.
    pub fn key(&mut self, key: Key) {
        if matches!(key, Key::Up | Key::Down) && !self.rows.is_empty() && self.checking.is_none() {
            self.step_pick(key == Key::Down);
            return;
        }
        if self.curtain && !self.engaged {
            self.opened = if key == Key::Enter { self.pick } else { None };
            self.open();
            self.lifted = true;
            return;
        }
        if !self.engaged || self.checking.is_some() || self.wait() > 0 {
            return;
        }
        match self.field.key(key) {
            Edit::Typed => self.wrong_words = None,
            Edit::Submit => self.submit(),
            Edit::Nothing => {}
        }
    }

    fn submit(&mut self) {
        let phc = match passcode::stored() {
            Some(phc) => phc,
            // The passcode taken off another way (over SSH): nothing to ask for.
            None if passcode::state() == Some(false) => {
                self.armed = false;
                self.open();
                return;
            }
            // The card would not be read just now: still locked; try again.
            None => {
                self.wrong_words = Some("could not read the card · try again".into());
                return;
            }
        };
        if self.field.len() < passcode::MIN_LEN {
            if !self.field.is_empty() {
                self.wrong_words = Some(format!("at least {} characters", passcode::MIN_LEN));
                self.field.wrong(false);
            }
            return;
        }
        let mut typed = self.field.take();
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("passcode".into()).stack_size(256 * 1024).spawn(move || {
            let right = passcode::check(&typed, &phc);
            passcode::wipe(&mut typed);
            tx.send(right).ok();
        });
        if spawned.is_ok() {
            self.checking = Some(rx);
        }
    }

    fn open(&mut self) {
        self.engaged = false;
        self.curtain = false;
        self.pick = None;
        self.entering = None;
        self.shown.set_with(0.0, role::release());
        self.field.clear();
        self.wrong_words = None;
        self.alert = None;
        self.shown.set(0.0);
    }

    /// Time passing (`dt` on the clock, `md` for motion): (moving, a new
    /// frame is wanted, it was just unlocked).
    pub fn tick(&mut self, dt: f32, md: f32, calm: bool, h24: bool) -> (bool, bool, bool) {
        self.refresh(dt);
        let mut unlocked = false;
        let mut changed = false;
        if self.engaged && !self.armed {
            // No passcode any more (taken off over SSH, the file damaged).
            self.open();
            unlocked = true;
        }
        // The curtain lifted: as good as unlocked, for what was said meanwhile.
        if std::mem::take(&mut self.lifted) {
            unlocked = true;
        }
        if let Some(t) = &mut self.entering {
            *t += md;
            if *t >= RISE_FOR || calm {
                self.entering = None;
            }
        }
        if let Some(rx) = &self.checking {
            match rx.try_recv() {
                Ok(true) => {
                    self.checking = None;
                    self.tries.passed();
                    // A notification picked before the passcode: it opens.
                    self.opened = self.pick;
                    self.open();
                    unlocked = true;
                }
                Ok(false) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.checking = None;
                    self.tries.failed(passcode::unix_now());
                    self.wrong_words = Some(match self.tries.before_wait() {
                        Some(1) => "wrong · 1 try before a wait".to_owned(),
                        Some(n) => format!("wrong · {n} tries before a wait"),
                        None => "not that one".to_owned(),
                    });
                    self.field.wrong(calm);
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if calm {
            self.shown.jump(self.shown.target());
        }
        let mut moving = self.shown.tick(md) | self.field.tick(md, calm);
        moving |= self.checking.is_some() || self.entering.is_some();
        // The robot between poses is motion; turning on its stand is not
        // (a frame wanted, the beat left to rest).
        moving |= self.robot.tick(md);
        if self.drifting() {
            self.robot.drift(dt);
            changed = true;
        }
        if !self.visible() {
            self.robot.shown = false;
            self.kept = None;
        }
        // A wait counting down, and the clock: a frame when either changes.
        let drawn = (self.wait(), time_text(crate::faces::local_now(), h24));
        if self.visible() && drawn != self.drawn {
            self.drawn = drawn;
            changed = true;
        }
        (moving, changed || moving || unlocked, unlocked)
    }

    fn prompt(&self) -> Prompt {
        let wait = self.wait();
        if self.curtain && self.pick.is_some() {
            Prompt::Asks("enter opens it".into())
        } else if self.curtain {
            Prompt::Curtain
        } else if !self.engaged {
            Prompt::Open
        } else if wait > 0 {
            Prompt::Wait(wait)
        } else if self.checking.is_some() {
            Prompt::Checking
        } else if let Some(words) = &self.wrong_words {
            Prompt::Wrong(words.clone())
        } else if let Some(alert) = &self.alert {
            Prompt::Alert(alert.clone())
        } else if self.pick.is_some() && self.field.is_empty() {
            Prompt::Asks("your passcode, and it opens".into())
        } else {
            Prompt::Asks("type your passcode".into())
        }
    }

    /// Draw over whatever is on the canvas: the lock screen, sliding up and
    /// away as it opens.
    pub fn draw(&mut self, g: &mut Gfx, choices: &Choices, info: &Info, scratch: &mut Vec<u32>) {
        let shown = self.shown.value().clamp(0.0, 1.0);
        if shown <= 0.004 && !self.covering() {
            return;
        }
        let fading = shown < 0.999;
        if fading {
            scratch.clone_from(&g.canvas.px);
        }
        let now = crate::faces::local_now();
        // The robot takes up the pose the handheld's state asks for.
        let pose = depth_pose(choices, info);
        if self.robot.aim(pose, ready_paint(pose, info), self.still || g.calm) {
            g.started_moving();
        }
        let shot = self.robot.shot();
        let prompt = self.prompt();
        // With the robot in front of the time, the picture under it (the
        // backdrop, the time, the date and the chips) is kept while it shows
        // the same, and the field's strip with it while the field is at
        // rest: a frame of the turntable is a copy and the robot laid over
        // it; a key in the field, a copy, the field and the robot.
        let scene = (!fading && info.rise.is_none()).then(|| Scene::of(g, choices, info, now, &shot));
        let field_key = (!self.field.moving()).then(|| (prompt.clone(), self.field.len(), self.field.width.target().round() as i32));
        if let (Some(s), Some(kept)) = (&scene, &mut self.kept) {
            if kept.scene == *s && kept.under.len() == g.canvas.px.len() {
                g.canvas.px.copy_from_slice(&kept.under);
                let strip = field_strip();
                match (&field_key, &kept.field) {
                    (Some(now), Some((was, px))) if now == was => g.canvas.px[strip].copy_from_slice(px),
                    _ => {
                        self.field.draw(g, kept.place.0, FIELD_Y, kept.place.1, &prompt);
                        kept.field = field_key.map(|k| (k, g.canvas.px[strip].to_vec()));
                    }
                }
                depth_robot(g, &shot, 0.0, 1.0);
                return;
            }
        }
        // The robot is laid over the rest last, after the field (whose glass
        // so never sees it, and keeps its own picture while the rest stands).
        let info = Info { pick: self.pick, depth: Some(shot), robot_apart: true, ..info.clone() };
        let layout = draw_face(g, choices, &info, now, false);
        self.robot.shown = layout.parts.iter().any(|(what, _)| what == "robot");
        let mut kept = match (scene, self.robot.shown) {
            (Some(scene), true) => {
                let mut under = self.kept.take().map(|k| k.under).unwrap_or_default();
                under.clone_from(&g.canvas.px);
                Some(Kept { scene, under, place: layout.field, field: None })
            }
            _ => None,
        };
        // What can be picked is what shows; one picked that went (opened
        // elsewhere, put away) is let go.
        self.rows = layout.rows.iter().map(|(id, _)| *id).collect();
        if self.pick.is_some_and(|id| !self.rows.contains(&id)) {
            self.pick = None;
        }
        let (cx, most) = layout.field;
        let (dy, _) = rise(&info, 5.0);
        self.field.draw(g, cx, FIELD_Y + dy.round(), most, &prompt);
        // The field's strip kept with the picture under the robot, and the
        // robot laid over them.
        if let Some(k) = &mut kept {
            k.field = field_key.map(|key| (key, g.canvas.px[field_strip()].to_vec()));
        }
        self.kept = kept;
        if self.robot.shown {
            let (dy, alpha) = rise(&info, 1.5);
            depth_robot(g, &shot, dy, alpha);
        }
        if fading {
            // Coming down over the screen in front, or opening: the lock
            // screen slides from or up and away to the top, revealing where
            // the handheld was, faded only near the end of its way (so the
            // two never stand over each other).
            let w = W as usize;
            let rows = H as usize;
            let lifted = (((1.0 - shown) * H).round().max(0.0) as usize).min(rows);
            let lock = std::mem::replace(&mut g.canvas.px, std::mem::take(scratch));
            let mut moved = g.canvas.px.clone();
            let edge = rows - lifted;
            // A soft shadow below the sheet's edge, on what it uncovers.
            for (i, row) in (edge..(edge + 8).min(rows)).enumerate() {
                let k = 0.32 * (1.0 - i as f32 / 8.0).powi(2);
                bezel::simd::mix_into(&mut moved[row * w..(row + 1) * w], None, (k * 256.0) as u32);
            }
            moved[..edge * w].copy_from_slice(&lock[lifted * w..]);
            // Coming down, the sheet is whole; going, it fades over the last
            // part of its way, gone before its field reaches the island (so
            // the field's glyph never stands over what the island says).
            let alpha = if self.covering() { 1.0 } else { ((shown - 0.25) * 2.2).clamp(0.0, 1.0) };
            bezel::simd::mix_into(&mut g.canvas.px, Some(&moved), (alpha * 256.0) as u32);
            *scratch = lock;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choices_read_back_and_default_to_no_words() {
        let c = Choices::default();
        assert!(c.standby && c.night && !c.previews, "StandBy on, notices' words hidden");
        assert!(Show::ALL.iter().all(|s| c.shows(*s)));
        let mut changed = Choices { standby: false, night: false, face: Face::Weather, clock: ClockStyle::Robot, lock_face: LockFace::Stacked, colour: LockColour::Ice, shows: 0, previews: true, depth: false, follows: false };
        changed.toggle(Show::Match);
        let mut back = Choices::default();
        for (k, v) in changed.lines() {
            assert!(back.parse(k, &v), "{k}");
        }
        assert_eq!(back, changed);
        assert!(!back.parse("light", "true"));
        back.parse("lock_face", "nonsense");
        assert_eq!(back.lock_face, LockFace::Stacked, "left as it was");
        assert_eq!(Face::Clock.step(-1), Face::Everything);
    }

    #[test]
    fn the_field_takes_signs_backspace_esc_and_enter() {
        let mut f = Field::new();
        for c in "12 34".chars() {
            f.key(Key::Char(c));
        }
        assert_eq!(f.len(), 4, "no spaces");
        assert_eq!(f.key(Key::Backspace), Edit::Typed);
        assert_eq!(f.len(), 3);
        assert_eq!(f.key(Key::Enter), Edit::Submit);
        assert_eq!(f.key(Key::Back), Edit::Typed);
        assert!(f.is_empty());
        assert_eq!(f.key(Key::Back), Edit::Nothing);
        for _ in 0..40 {
            f.key(Key::Char('9'));
        }
        assert_eq!(f.len(), passcode::MAX_LEN);
        assert_eq!(f.take().len(), passcode::MAX_LEN);
        assert!(f.is_empty());
    }

    /// Notifications as they come: the newest first.
    fn notes(n: usize) -> Vec<crate::notify::Notification> {
        let all = [
            ("Schedule", "flag", "time to queue for Q31", "red · with 1507, 5254"),
            ("Reminders", "checklist", "swap the battery", "after Q31"),
            ("Clock", "timer", "timer 5 min done", ""),
            ("Music", "download_done", "12 songs saved", "in the library, to play offline"),
        ];
        all.iter().cycle().take(n).enumerate().map(|(i, (app, icon, title, body))| crate::notify::Notification { id: i as u64 + 1, at: 1_000 - i as i64 * 60, ..crate::notify::Notification::new(app, icon, *title).body(*body) }).collect()
    }

    fn info(notices: usize) -> Info {
        let robot = crate::robot::Robot::demo();
        let mut info = crate::lockpages::info_now(&robot, false, Some(Reading::demo()));
        info.notes = notes(notices);
        info.notices = notices;
        info
    }

    #[test]
    fn notifications_words_wait_until_unlocked_unless_chosen() {
        let c = Choices::default();
        let words: Vec<String> = items(&c, &info(2), 0, false).into_iter().map(|i| i.3).collect();
        assert!(words.contains(&"2 notifications".to_owned()), "{words:?}");
        assert!(!words.iter().any(|w| w.contains("queue")), "no words by default: {words:?}");
        let shown = Choices { previews: true, ..c };
        let words: Vec<String> = items(&shown, &info(2), 0, false).into_iter().map(|i| i.3).collect();
        assert!(words.contains(&"time to queue for Q31 · +1".to_owned()), "{words:?}");
        // None, none shown (the editor's preview says so).
        assert!(!items(&c, &info(0), 0, false).iter().any(|i| i.0 == Show::Notices));
        assert!(items(&c, &info(0), 0, true).iter().any(|i| i.0 == Show::Notices));
    }

    #[test]
    fn every_lock_face_fits_whole_above_its_field_at_every_text_size() {
        crate::shell::tests::sandbox();
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap().and_hms_opt(12, 59, 0).unwrap();
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            let mut g = Gfx::new(bezel::Tokens::dark());
            g.set_text_size(size);
            for face in LockFace::ALL {
                for (editing, previews) in [(false, false), (false, true), (true, false)] {
                    let c = Choices { lock_face: *face, previews, ..Choices::default() };
                    // Counted only (their rows are the band's: tested below).
                    let info = Info { notes: Vec::new(), ..info(3) };
                    let layout = draw_face(&mut g, &c, &info, now, editing);
                    let time = layout.places.iter().find(|(p, _)| *p == Part::Time).expect("the time").1;
                    let (cx, most) = layout.field;
                    assert!(cx - most * 0.5 >= PAD - 0.5 && cx + most * 0.5 <= W - PAD + 0.5);
                    let field = Rect::new(cx - most * 0.5, FIELD_Y - 2.0, most, FIELD_H + 2.0);
                    let apart = |a: &Rect, b: &Rect| a.right() <= b.x || a.x >= b.right() || a.bottom() <= b.y || a.y >= b.bottom();
                    for (part, r) in &layout.places {
                        assert!(r.x >= 0.0 && r.right() <= W && r.y >= 0.0 && r.bottom() <= H, "{part:?} at {r:?}, {face:?} {size:?}");
                        assert!(apart(r, &field), "{part:?} {r:?} over the field, {face:?} {size:?}");
                        if *part != Part::Time {
                            assert!(apart(r, &time), "{part:?} {r:?} over the time {time:?}, {face:?} {size:?}");
                        }
                    }
                    // Every chip shown when editing; the notices always.
                    assert!(layout.places.iter().any(|(p, _)| *p == Part::Item(Show::Notices)), "{face:?} {size:?} {editing}");
                }
            }
            // The field's words fit it, grown, at this size.
            let mut field = Field::new();
            for prompt in [Prompt::Asks("type your passcode".into()), Prompt::Wrong("wrong · 2 tries before a wait".into()), Prompt::Wait(3599), Prompt::Alert("battery empty · turning off".into())] {
                let (_, words, _) = Field::words(&g, &prompt);
                assert!(g.measure(&words, g.ty.caption) + 44.0 <= W - 2.0 * PAD - 12.0, "{words} at {size:?}");
                field.draw(&mut g, W * 0.5, FIELD_Y, W - 2.0 * PAD - 12.0, &prompt);
            }
            assert!(g.missing_icons().is_empty(), "{:?}", g.missing_icons());
        }
    }

    /// The robot stands in front of the lower part of the time and never
    /// more than its lower third (the hours and minutes read), on the
    /// centred and dotted clocks, at every text size, in every pose, on a
    /// 12- or 24-hour clock; it reaches the time, and nothing else (the
    /// date, the chips and the field are tested apart in the next test).
    /// Its extent at every angle is held by depth.rs's own test.
    #[test]
    fn the_robot_covers_at_most_the_lower_third_of_the_time() {
        crate::shell::tests::sandbox();
        use crate::depth::{Pose, Shot, MOST_COVER};
        let mut checked = 0;
        for (hm, h24) in [((9, 41), false), ((12, 59), false), ((23, 58), true)] {
            let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap().and_hms_opt(hm.0, hm.1, 0).unwrap();
            for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
                let mut g = Gfx::new(bezel::Tokens::dark());
                g.set_text_size(size);
                for face in [LockFace::Centre, LockFace::Dots] {
                    for pose in Pose::ALL {
                        let c = Choices { lock_face: face, ..Choices::default() };
                        let mut info = info(0);
                        info.h24 = h24;
                        info.depth = Some(Shot::settled(pose, None));
                        let layout = draw_face(&mut g, &c, &info, now, false);
                        let find = |what: &str| layout.parts.iter().find(|(n, _)| n == what).map(|(_, r)| *r).unwrap_or_else(|| panic!("no {what}: {face:?} {size:?}"));
                        let (time, robot) = (find("time"), find("robot"));
                        let covered = (time.bottom() - robot.y) / time.h;
                        let case = format!("{face:?} {} {pose:?} {}:{:02}", size.name(), hm.0, hm.1);
                        assert!(covered <= MOST_COVER + 1e-3, "covers {covered:.2} of the time: {case}");
                        assert!(covered >= 0.18, "hardly in front ({covered:.2}): {case}");
                        assert!(robot.x > time.x && robot.right() < time.right(), "in front of the time, not beside it: {case}");
                        assert!(time.x >= PAD - 0.5 && time.right() <= W - PAD + 0.5, "the time on the panel: {case}");
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 72);
        // Off, or on the stacked clock: no robot.
        let mut g = Gfx::new(bezel::Tokens::dark());
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap().and_hms_opt(9, 41, 0).unwrap();
        for c in [Choices { depth: false, ..Choices::default() }, Choices { lock_face: LockFace::Stacked, ..Choices::default() }] {
            assert!(!draw_face(&mut g, &c, &info(0), now, false).parts.iter().any(|(n, _)| n == "robot"));
        }
    }

    /// The live activities a lock screen may show, the most important first.
    fn activities(n: usize) -> Vec<crate::liveactivity::Activity> {
        let song = crate::nowplaying::NowPlaying {
            title: "Around the World".into(),
            artist: "Daft Punk".into(),
            album: "Homework".into(),
            cover: None,
            position: 80.0,
            duration: 429.0,
            playing: true,
            at: std::time::Instant::now(),
        };
        let mut all = crate::liveactivity::timers(Some((272.0, 300.0, 1)), None);
        all.extend(crate::liveactivity::music(Some(&song)));
        all.extend(crate::liveactivity::reminder(Some(&crate::upnext::Reminder { title: "swap the battery".into(), due: Some(chrono::Utc::now().timestamp() + 600) }), chrono::Utc::now().timestamp()));
        all.truncate(n);
        all
    }

    /// Thomas's: "the battery charging thing clips with the music thing".
    /// Charging or not, no activity to three, no notification to four, the
    /// words shown or not, every face, every text size, both tones, with
    /// the field (locked, or the curtain) and without it (the editor): no
    /// two things drawn on the lock screen meet, and all are on the panel.
    #[test]
    fn nothing_on_the_lock_screen_runs_into_anything_else() {
        crate::shell::tests::sandbox();
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap().and_hms_opt(12, 59, 0).unwrap();
        let charging = Reading { flow: crate::power::Flow::Charging, minutes_to_full: Some(42), ..Reading::demo() };
        let mut checked = 0;
        let mut wrong = Vec::new();
        for tokens in [bezel::Tokens::dark(), bezel::Tokens::light()] {
            for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
                let mut g = Gfx::new(tokens);
                g.set_text_size(size);
                for face in LockFace::ALL {
                    for reading in [Reading::demo(), charging.clone()] {
                        for acts in 0..=3 {
                            for notes in [0, 1, 4] {
                                for previews in [false, true] {
                                    for (editing, prompt) in [(false, Some(Prompt::Asks("your passcode, and it opens".into()))), (false, Some(Prompt::Curtain)), (true, None)] {
                                        let c = Choices { lock_face: *face, previews, ..Choices::default() };
                                        let mut info = info(notes);
                                        info.reading = Some(reading.clone());
                                        info.activities = activities(acts);
                                        let layout = draw_face(&mut g, &c, &info, now, editing);
                                        let mut parts = layout.parts.clone();
                                        if let Some(prompt) = prompt {
                                            let (cx, most) = layout.field;
                                            let mut field = Field::new();
                                            field.draw(&mut g, cx, FIELD_Y, most, &prompt);
                                            let w = field.width.value();
                                            parts.push(("field".into(), Rect::new(cx - w * 0.5, FIELD_Y, w, FIELD_H)));
                                        }
                                        let case = format!("{face:?} {} {:?} charging {} acts {acts} notes {notes} words {previews} editing {editing}", size.name(), tokens.tone, !reading.flow.on_battery());
                                        for (i, (a, ra)) in parts.iter().enumerate() {
                                            if ra.x < 0.0 || ra.right() > W || ra.y < 0.0 || ra.bottom() > H {
                                                wrong.push(format!("{a} {ra:?} off the panel: {case}"));
                                            }
                                            for (b, rb) in &parts[i + 1..] {
                                                // The robot stands in front of the time by design (depth.rs).
                                                if [a.as_str(), b.as_str()] == ["time", "robot"] || [a.as_str(), b.as_str()] == ["robot", "time"] {
                                                    continue;
                                                }
                                                let apart = ra.right() + 2.0 <= rb.x || ra.x >= rb.right() + 2.0 || ra.bottom() + 2.0 <= rb.y || ra.y >= rb.bottom() + 2.0;
                                                if !apart {
                                                    wrong.push(format!("{a} {ra:?} meets {b} {rb:?}: {case}"));
                                                }
                                            }
                                        }
                                        checked += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(checked > 1000, "only {checked}");
        assert!(wrong.is_empty(), "{} of {checked}:\n{}", wrong.len(), wrong.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
    }

    #[test]
    fn a_notification_picked_on_the_lock_screen_opens_once_unlocked() {
        let mut lock = Lock::new();
        lock.curtain = true;
        lock.rows = vec![7, 9];
        // ↓ to the first, ↓ to the next, ↓ past the last: none; ↑ from none: the last.
        lock.key(Key::Down);
        assert_eq!(lock.pick(), Some(7));
        lock.key(Key::Down);
        assert_eq!(lock.pick(), Some(9));
        lock.key(Key::Down);
        assert_eq!(lock.pick(), None);
        lock.key(Key::Up);
        assert_eq!(lock.pick(), Some(9));
        assert!(lock.covering(), "picking lifts nothing");
        assert!(matches!(lock.prompt(), Prompt::Asks(w) if w == "enter opens it"));
        // Enter on the curtain: lifted, and that one to open.
        lock.key(Key::Enter);
        assert!(!lock.covering());
        assert_eq!(lock.take_opened(), Some(9));
        assert_eq!(lock.take_opened(), None, "once");
        // Any other key lifts it with nothing to open.
        lock.curtain = true;
        lock.rows = vec![7];
        lock.key(Key::Down);
        lock.key(Key::Char('a'));
        assert!(!lock.covering() && lock.take_opened().is_none());
    }

    #[test]
    fn a_wrong_passcode_shakes_the_field_and_it_settles() {
        let mut f = Field::new();
        f.key(Key::Char('1'));
        f.wrong(false);
        assert!(f.is_empty());
        let mut most: f32 = 0.0;
        let mut swings = 0;
        let mut last = 0.0f32;
        for _ in 0..60 {
            f.tick(1.0 / 60.0, false);
            let x = f.shake.value();
            most = most.max(x.abs());
            if x.signum() != last.signum() && x.abs() > 0.5 {
                swings += 1;
            }
            last = x;
        }
        assert!((6.0..=11.0).contains(&most), "a shake you see, not a jolt: {most}");
        assert!((3..=8).contains(&swings), "{swings}");
        assert!(f.shake.value().abs() < 0.3, "settled within a second");
    }
}
