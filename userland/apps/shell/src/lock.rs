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
//! - **Notices** said while it is locked are kept for it. By default their
//!   words wait until it is unlocked (Apple's "when unlocked", and the safe
//!   choice for a handheld on a desk in the pits): the lock screen says how
//!   many, and when; Settings › Lock screen can show the words.
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

choice!(Face { Clock => "clock", Charge => "charge", Match => "next match", Weather => "weather", Photos => "photos", Everything => "everything" });
choice!(ClockStyle { Digits => "digits", Dial => "dial", Dots => "dots" });
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
    pub fn lines(&self) -> [(&'static str, String); 8] {
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
        ]
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

impl Default for Field {
    fn default() -> Self {
        Self::new()
    }
}

impl Field {
    pub fn new() -> Field {
        Field {
            text: String::new(),
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
    pub fn take(&mut self) -> String {
        let text = std::mem::take(&mut self.text);
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
    /// Notices since it locked: how many, the newest's words, and when.
    pub notices: usize,
    pub newest: Option<(String, i64)>,
}

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
                    0 => "no notices".to_owned(),
                    1 => "1 notice".to_owned(),
                    n => format!("{n} notices"),
                };
                match (info.notices, &info.newest) {
                    (0, _) if !editing => None,
                    (1, Some((text, _))) if choices.previews => Some((Some("notifications"), None, text.clone(), Some(count))),
                    (n, Some((text, _))) if choices.previews && n > 1 => Some((Some("notifications"), None, format!("{text} · +{}", n - 1), Some(count))),
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

/// Lay the chips in rows `width` wide from `x` (centred on each row, or
/// from its left), the first row's middle at `cy`, as many rows as fit
/// above the field: each whole on its row (a long one, the notices' words,
/// cut to the row), in order, as many as fit. Records where each went.
#[allow(clippy::too_many_arguments)]
fn flow(g: &mut Gfx, chips: &[Item], x: f32, width: f32, cy: f32, centred: bool, places: &mut Vec<(Part, Rect)>) {
    let t = g.tokens;
    let (gap, pitch) = (6.0, (g.ty.caption.size + 9.0).round() + 5.0);
    let rows = (((FIELD_Y - 4.0) - (cy - (pitch - 5.0) * 0.5) + 5.0) / pitch).floor().max(1.0) as usize;
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
        let mut at = if centred { x + (width - total) * 0.5 } else { x };
        let y = cy + row as f32 * pitch;
        for (i, words, w) in line {
            let (what, icon, dot, _, _, on) = &chips[*i];
            let alpha = if *on { 1.0 } else { 0.3 };
            chip(g, at.round(), y, *icon, *dot, words, t.surface2, t.ink, alpha);
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
}

/// Draw the lock screen's face (all but the field) as `choices` have it.
/// `editing`: the editor's preview, where what is switched off still shows,
/// faintly.
///
/// - **Centre**: the date, the time large in the middle, the chips beneath
///   in up to two rows, the field low and centred.
/// - **Dots**: the same, the time set in dots.
/// - **Stacked**: the hours over the minutes, large, down the left; the
///   date, the chips and the field down the right.
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
    let chips = items(choices, info, unix, editing);
    match choices.lock_face {
        LockFace::Centre | LockFace::Dots => {
            let date_cy = 15.0;
            if date_on || editing {
                let date = date_text(now);
                let date = if g.measure(&date, ty.caption) > W - 2.0 * PAD { short_date(now) } else { date };
                let w = g.measure(&date, ty.caption);
                g.text.draw_aligned(&mut g.canvas, W * 0.5, (date_cy + ty.caption.size * 0.36).round(), Align::Center, &date, &ty.caption, t.dim, date_alpha);
                places.push((Part::Date, Rect::new((W - w) * 0.5 - 6.0, date_cy - 8.0, w + 12.0, 16.0)));
            }
            let below = if choices.lock_face == LockFace::Dots {
                let (pitch, top) = (6.5, 32.0);
                let (cols, _) = crate::faces::dot_time(&time);
                draw_dots(g, &time, &time, 1.0, W * 0.5, top, pitch, tint, 1.0);
                let w = cols as f32 * pitch;
                places.push((Part::Time, Rect::new((W - w) * 0.5 - 4.0, top - 6.0, w + 8.0, 6.0 * pitch + 12.0)));
                top + 6.0 * pitch + 19.0
            } else {
                let style = ty.display(54.0);
                let tab = Tab::new(g, style);
                let cap = g.text.cap_height(&style);
                let w = tab.width(&time);
                let baseline = (27.0 + cap).round();
                tab.draw(g, &time, &time, 1.0, (W - w) * 0.5, baseline, tint, 1.0);
                places.push((Part::Time, Rect::new((W - w) * 0.5 - 6.0, baseline - cap - 3.0, w + 12.0, cap + 6.0)));
                baseline + 19.0
            };
            flow(g, &chips, PAD, W - 2.0 * PAD, below, true, &mut places);
            Layout { places, field: (W * 0.5, W - 2.0 * PAD - 12.0) }
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
            tab.draw(g, hours, hours, 1.0, x, first, tint, 1.0);
            tab.draw(g, minutes, minutes, 1.0, x, second, tint, 1.0);
            let w = tab.width("88");
            places.push((Part::Time, Rect::new(x - 6.0, top - 4.0, w + 12.0, 2.0 * cap + gap + 8.0)));
            // The rest down the right.
            let col = x + w + 16.0;
            let width = W - PAD - 2.0 - col;
            if date_on || editing {
                let date = date_text(now);
                let date = if g.measure(&date, ty.caption) > width { short_date(now) } else { date };
                let dw = g.text.draw(&mut g.canvas, col, (top + 6.0 + ty.caption.size * 0.36).round(), &date, &ty.caption, t.dim, date_alpha);
                places.push((Part::Date, Rect::new(col - 6.0, top - 2.0, dw + 12.0, 16.0)));
            }
            flow(g, &chips, col, width, top + 30.0, false, &mut places);
            Layout { places, field: (col + width * 0.5, width) }
        }
    }
}

// ------------------------------------------------------------- the layer ---

/// At most this many notices are kept for the lock screen.
const NOTICES_KEPT: usize = 20;

/// The lock, over everything while it is engaged.
pub struct Lock {
    /// Locked: only the passcode opens it.
    engaged: bool,
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
    notices: Vec<(String, i64)>,
    /// A warning that outranks the lock (the battery running out).
    alert: Option<String>,
    /// The second last drawn of a wait, and the minute of the clock.
    drawn: (u64, String),
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
            armed,
            armed_generation: passcode::generation(),
            armed_age: 0.0,
            shown: Animated::new(if armed { 1.0 } else { 0.0 }, role::release()).with_rest(0.004),
            field: Field::new(),
            checking: None,
            tries: Tries::load(passcode::unix_now()),
            wrong_words: None,
            notices: Vec::new(),
            alert: None,
            drawn: (0, String::new()),
        }
    }

    /// A passcode is set.
    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn engaged(&self) -> bool {
        self.engaged
    }

    /// Anything of the lock screen is on the panel.
    pub fn visible(&self) -> bool {
        self.engaged || self.shown.value() > 0.004
    }

    /// It covers the panel whole: nothing beneath need be drawn.
    pub fn covers(&self) -> bool {
        self.engaged && self.shown.value() >= 0.999
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
        self.shown.jump(1.0);
        self.field.clear();
        self.wrong_words = None;
        self.tries = Tries::load(passcode::unix_now());
    }

    /// Kept for the lock screen: something was said while it was locked.
    pub fn note(&mut self, text: &str) {
        self.notices.push((text.to_owned(), passcode::unix_now()));
        if self.notices.len() > NOTICES_KEPT {
            self.notices.remove(0);
        }
    }

    pub fn notices(&self) -> usize {
        self.notices.len()
    }

    /// The notices kept, newest last, handed over once it is unlocked.
    pub fn take_notices(&mut self) -> Vec<(String, i64)> {
        std::mem::take(&mut self.notices)
    }

    pub fn newest(&self) -> Option<(String, i64)> {
        self.notices.last().cloned()
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

    /// A key on the lock screen.
    pub fn key(&mut self, key: Key) {
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
        if let Some(rx) = &self.checking {
            match rx.try_recv() {
                Ok(true) => {
                    self.checking = None;
                    self.tries.passed();
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
        moving |= self.checking.is_some();
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
        if !self.engaged {
            Prompt::Open
        } else if wait > 0 {
            Prompt::Wait(wait)
        } else if self.checking.is_some() {
            Prompt::Checking
        } else if let Some(words) = &self.wrong_words {
            Prompt::Wrong(words.clone())
        } else if let Some(alert) = &self.alert {
            Prompt::Alert(alert.clone())
        } else {
            Prompt::Asks("type your passcode".into())
        }
    }

    /// Draw over whatever is on the canvas: the lock screen, sliding up and
    /// away as it opens.
    pub fn draw(&mut self, g: &mut Gfx, choices: &Choices, info: &Info, scratch: &mut Vec<u32>) {
        let shown = self.shown.value().clamp(0.0, 1.0);
        if shown <= 0.004 && !self.engaged {
            return;
        }
        let fading = shown < 0.999;
        if fading {
            scratch.clone_from(&g.canvas.px);
        }
        let now = crate::faces::local_now();
        let layout = draw_face(g, choices, info, now, false);
        let (cx, most) = layout.field;
        let prompt = self.prompt();
        self.field.draw(g, cx, FIELD_Y, most, &prompt);
        if fading {
            // Opening: the lock screen slides up and away, revealing where
            // the handheld was, fading only as it goes (so the two never
            // stand over each other).
            let w = W as usize;
            let rows = H as usize;
            let lifted = (((1.0 - shown) * H).round().max(0.0) as usize).min(rows);
            let lock = std::mem::replace(&mut g.canvas.px, std::mem::take(scratch));
            let mut moved = g.canvas.px.clone();
            moved[..(rows - lifted) * w].copy_from_slice(&lock[lifted * w..]);
            let alpha = (shown * 1.6).clamp(0.0, 1.0);
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
        let mut changed = Choices { standby: false, night: false, face: Face::Weather, clock: ClockStyle::Dots, lock_face: LockFace::Stacked, colour: LockColour::Ice, shows: 0, previews: true };
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

    fn info(notices: usize) -> Info {
        let robot = crate::robot::Robot::demo();
        let mut info = crate::lockpages::info_now(&robot, false, Some(Reading::demo()));
        info.notices = notices;
        info.newest = (notices > 0).then(|| ("time to queue for Q31".to_owned(), 0));
        info
    }

    #[test]
    fn notices_words_wait_until_unlocked_unless_chosen() {
        let c = Choices::default();
        let words: Vec<String> = items(&c, &info(2), 0, false).into_iter().map(|i| i.3).collect();
        assert!(words.contains(&"2 notices".to_owned()), "{words:?}");
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
                    let layout = draw_face(&mut g, &c, &info(3), now, editing);
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
