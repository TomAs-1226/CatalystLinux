//! The Camera's controls in sight (1.1): a bar of chips along the
//! viewfinder's foot, so nothing on the camera needs remembering.
//!
//! The chips: the mode, the look, the self-timer, the grid, detail and pro's
//! controls, and the lens with a smart camera (in scan and doc modes, and in
//! the AI view, only the mode and the lens). Each wears its key, so the keys
//! are learnt by using the bar, never needed for it.
//!
//! The keys, for a keyboard handheld: ← → stay the mode's, as the rail shows
//! them, and Enter and Space the shutter. Tab takes the keys into the bar,
//! on the chip last used: ← → (or Tab) go along it, Enter opens the chip's
//! choices, Esc or ↑ gives the keys back to the camera. Space is the shutter
//! even then. The choices sit in the bar's place, one capsule of ice on the
//! one in hand: ← → choose, Enter sets it, Esc leaves it as it was. The look
//! opens the strip of looks (live thumbnails, named; Enter keeps, Esc goes
//! back); pro opens a row of its own, Shutter to Presets, each turned with
//! ← → and the next with ↑ ↓ or Tab.
//!
//! The shutter sends the bar down out of the way, and it comes back at the
//! next key that is not the shutter. Its glass comes and goes by its
//! refraction, its words with it; at rest nothing in it moves.
//!
//! Pixel mode (1.2, camera/pixel_mode.rs) has chips of its own: the size
//! (`s`), the palette (`k`, its colours on the chip and in its picker), the
//! dither (`d`, its pattern on the chip) and the tone (↑ ↓ the brightness,
//! its picker the contrast), then the timer. Their pickers show each choice
//! on the picture as it is in hand; Enter keeps it, Esc goes back.

use super::*;
use crate::burst::DetailMode;

/// A chip of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Mode,
    Look,
    Timer,
    Grid,
    Detail,
    Pro,
    Lens,
    /// The automatic exposure's (1.3.1): Night, HDR and the lock.
    Night,
    Hdr,
    Lock,
    /// Pixel mode's (1.2).
    Size,
    Palette,
    Dither,
    Tone,
}

impl Chip {
    pub(super) fn name(self) -> &'static str {
        match self {
            Chip::Mode => "mode",
            Chip::Look => "look",
            Chip::Timer => "timer",
            Chip::Grid => "grid",
            Chip::Detail => "detail",
            Chip::Pro => "pro",
            Chip::Lens => "lens",
            Chip::Night => "night",
            Chip::Hdr => "hdr",
            Chip::Lock => "lock",
            Chip::Size => "size",
            Chip::Palette => "palette",
            Chip::Dither => "dither",
            Chip::Tone => "tone",
        }
    }

    /// Its key, worn on the chip.
    pub fn badge(self) -> &'static str {
        match self {
            Chip::Mode => "←→",
            Chip::Look => "k",
            Chip::Timer => "t",
            Chip::Grid => "g",
            Chip::Detail => "d",
            Chip::Pro => "p",
            Chip::Lens => "l",
            Chip::Night => "n",
            Chip::Hdr => "b",
            Chip::Lock => "e",
            Chip::Size => "s",
            Chip::Palette => "k",
            Chip::Dither => "d",
            Chip::Tone => "↑↓",
        }
    }

    /// One of pixel mode's chips.
    pub fn pixel(self) -> bool {
        matches!(self, Chip::Size | Chip::Palette | Chip::Dither | Chip::Tone)
    }
}

/// Pro's controls, in their row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProItem {
    Shutter,
    Iso,
    Ev,
    Wb,
    Raw,
    Zebra,
    Histogram,
    Bracket,
    Priority,
    Presets,
}

pub const PRO_ITEMS: [ProItem; 10] = [ProItem::Shutter, ProItem::Iso, ProItem::Ev, ProItem::Wb, ProItem::Raw, ProItem::Zebra, ProItem::Histogram, ProItem::Bracket, ProItem::Priority, ProItem::Presets];

impl ProItem {
    fn name(self) -> &'static str {
        match self {
            ProItem::Shutter => "shutter",
            ProItem::Iso => "iso",
            ProItem::Ev => "ev",
            ProItem::Wb => "wb",
            ProItem::Raw => "raw",
            ProItem::Zebra => "zebra",
            ProItem::Histogram => "histogram",
            ProItem::Bracket => "bracket",
            ProItem::Priority => "priority",
            ProItem::Presets => "presets",
        }
    }

    fn badge(self) -> &'static str {
        match self {
            ProItem::Shutter => "s",
            ProItem::Iso => "i",
            ProItem::Ev => "e",
            ProItem::Wb => "w",
            ProItem::Raw => "r",
            ProItem::Zebra => "q",
            ProItem::Histogram => "u",
            ProItem::Bracket => "b",
            ProItem::Priority => "m",
            ProItem::Presets => "1-3",
        }
    }

    fn dial(self) -> Option<Dial> {
        match self {
            ProItem::Shutter => Some(Dial::Shutter),
            ProItem::Iso => Some(Dial::Iso),
            ProItem::Ev => Some(Dial::Ev),
            ProItem::Wb => Some(Dial::Wb),
            _ => None,
        }
    }

    /// The item a key of pro mode's is for.
    fn of_key(c: char) -> Option<ProItem> {
        PRO_ITEMS.iter().copied().find(|i| i.badge() == c.to_string()).or_else(|| matches!(c, '1'..='3' | '!' | '@' | '#').then_some(ProItem::Presets))
    }
}

/// What is open in the bar's place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Picker {
    /// A chip's choices, `at` the one in hand.
    Choice { chip: Chip, at: usize },
    /// Pro's row, on `item`; Presets' slot in hand.
    Pro { item: usize, slot: usize },
}

pub struct Bar {
    /// The chip with the keys (Tab), if the bar has them.
    pub focus: Option<Chip>,
    pub picker: Option<Picker>,
    /// Sent down while photos are taken.
    pub hidden: bool,
    /// The chip Tab comes back to.
    pub(super) last: Chip,
    shown: Animated,
    /// The capsule of ice: where it is along the cells, and how far in.
    pub(super) at: Animated,
    pub(super) on: Animated,
    /// Pro's row, slid along to keep the item in hand in sight.
    pub(super) scroll: Animated,
    /// The chips, slid along the same way where they are more than fit
    /// (1.3.1: Night, HDR and the lock joined them).
    pub(super) slide: Animated,
    placed: bool,
}

impl Bar {
    pub fn new() -> Bar {
        Bar {
            focus: None,
            picker: None,
            hidden: false,
            last: Chip::Look,
            // The glass and its words on the effects spring (as the dials'
            // fade); the capsule on Bezel's mode buttons' (the dials' mark).
            shown: Animated::new(1.0, role::effect()).with_rest(0.002),
            at: Animated::new(0.0, role::release()).with_rest(0.002),
            on: Animated::new(0.0, role::effect()).with_rest(0.002),
            scroll: Animated::new(0.0, role::release()).with_rest(0.05),
            slide: Animated::new(0.0, role::release()).with_rest(0.05),
            placed: false,
        }
    }

    pub fn tick(&mut self, dt: f32) -> bool {
        self.shown.tick(dt) | self.at.tick(dt) | self.on.tick(dt) | self.scroll.tick(dt) | self.slide.tick(dt)
    }

    /// Whether the bar holds the keys (a chip, or what one opened).
    pub fn has_keys(&self) -> bool {
        self.focus.is_some() || self.picker.is_some()
    }

    pub fn pro_open(&self) -> bool {
        matches!(self.picker, Some(Picker::Pro { .. }))
    }
}

impl Camera {
    /// The chips along the bar, as the camera is now.
    pub(super) fn chips(&self) -> Vec<Chip> {
        if self.quick {
            return vec![Chip::Night, Chip::Timer, Chip::Grid, Chip::Detail];
        }
        let mut chips = vec![Chip::Mode];
        // The automatic exposure's first, by the mode (1.3.1): Night for
        // photo and pro, HDR and the lock for photo mode alone.
        if self.lens == Lens::Main && self.mode == Mode::Photo {
            chips.extend([Chip::Night, Chip::Hdr, Chip::Lock]);
        }
        if self.lens == Lens::Main && self.mode == Mode::Pro {
            chips.push(Chip::Night);
        }
        if self.lens == Lens::Main && self.mode.shoots() {
            chips.extend([Chip::Look, Chip::Timer, Chip::Grid, Chip::Detail, Chip::Pro]);
        }
        if self.lens == Lens::Main && self.mode == Mode::Pixel {
            chips.extend([Chip::Size, Chip::Palette, Chip::Dither, Chip::Tone, Chip::Timer]);
        }
        if self.smart.is_some() {
            chips.push(Chip::Lens);
        }
        chips
    }

    /// A key for the bar, before the camera's own: `Some` if it took it.
    pub(super) fn bar_key(&mut self, key: Key, cx: &mut Cx) -> Option<Nav> {
        // The shutter: photos taken, the bar steps aside (a chip's choices
        // stay, to be changed between photos). Space is the shutter even
        // in the bar; Enter is, outside it.
        let in_hand = self.bar.has_keys() || self.strip;
        let shutter = matches!(key, Key::Char(' ') | Key::Ctrl('p' | 'P')) || (key == Key::Enter && !in_hand);
        if shutter {
            if self.bar.picker.is_none() {
                self.bar.focus = None;
                self.bar.hidden = true;
            }
            return None;
        }
        // Any other key brings it back.
        self.bar.hidden = false;
        if let Some(picker) = self.bar.picker {
            if let Picker::Choice { chip, at } = picker {
                if chip.pixel() {
                    return self.pixel_picker_key(chip, at, key);
                }
            }
            return self.picker_key(picker, key, cx);
        }
        // The looks' strip, opened from the bar, has the keys.
        if self.strip {
            return None;
        }
        if let Some(chip) = self.bar.focus {
            let chips = self.chips();
            let Some(at) = chips.iter().position(|c| *c == chip) else {
                self.bar.focus = None;
                return None;
            };
            let to = |c: &mut Camera, i: usize| {
                c.bar.focus = Some(chips[i]);
                c.bar.last = chips[i];
            };
            match key {
                Key::Left if at > 0 => to(self, at - 1),
                Key::Right if at + 1 < chips.len() => to(self, at + 1),
                Key::Left | Key::Right => self.bar.at.nudge(if key == Key::Right { 3.0 } else { -3.0 }),
                Key::Tab => to(self, (at + 1) % chips.len()),
                Key::Enter => self.open_chip(chip, cx),
                Key::Back | Key::Up => self.bar.focus = None,
                // Down there is nowhere to go.
                Key::Down => {}
                // Any other key is the camera's, the bar keeping the keys.
                _ => return None,
            }
            return Some(Nav::Stay);
        }
        if key == Key::Tab && self.focus.is_none() {
            let chips = self.chips();
            let chip = if chips.contains(&self.bar.last) { self.bar.last } else { chips[0] };
            self.set_dial(None);
            self.bar.focus = Some(chip);
            return Some(Nav::Stay);
        }
        None
    }

    /// Enter on a chip: its choices, the looks, pro's row, or the other lens.
    fn open_chip(&mut self, chip: Chip, cx: &mut Cx) {
        match chip {
            Chip::Look => self.set_strip(true),
            Chip::Pro => self.open_pro_row(),
            Chip::Lens => {
                self.set_lens(if self.lens == Lens::Main { Lens::Ai } else { Lens::Main });
                // The chips change with the lens: the keys stay on it.
                self.bar.focus = Some(Chip::Lens);
            }
            // The lock has no choices: Enter locks or lets go, as `e` does.
            Chip::Lock => self.toggle_lock(),
            c if c.pixel() => self.open_pixel_picker(c),
            _ => {
                let at = self.chosen(chip, cx.settings.detail);
                self.bar.at.jump(at as f32);
                self.bar.picker = Some(Picker::Choice { chip, at });
                self.say("← → · enter: set · esc: as it was");
            }
        }
    }

    /// Pro's row, in pro mode (going to it from another).
    pub(super) fn open_pro_row(&mut self) {
        if self.quick || self.lens != Lens::Main {
            return;
        }
        if self.mode != Mode::Pro {
            self.set_mode(Mode::Pro);
        }
        self.set_strip(false);
        self.set_dial(None);
        self.bar.hidden = false;
        self.bar.focus = Some(Chip::Pro);
        self.bar.last = Chip::Pro;
        let item = match self.bar.picker {
            Some(Picker::Pro { item, .. }) => item,
            _ => 0,
        };
        self.bar.at.jump(item as f32);
        self.bar.picker = Some(Picker::Pro { item, slot: 0 });
        self.say("← → change · ↑ ↓ or tab: next · esc");
    }

    fn picker_key(&mut self, picker: Picker, key: Key, cx: &mut Cx) -> Option<Nav> {
        match picker {
            Picker::Choice { chip, at } => {
                let len = self.choices(chip).len();
                match key {
                    Key::Left | Key::Right => {
                        let to = if key == Key::Right { (at + 1).min(len - 1) } else { at.saturating_sub(1) };
                        if to == at {
                            self.bar.at.nudge(if key == Key::Right { 3.0 } else { -3.0 });
                        }
                        self.bar.picker = Some(Picker::Choice { chip, at: to });
                    }
                    Key::Enter => {
                        self.bar.picker = None;
                        self.apply_choice(chip, at, cx);
                    }
                    Key::Back | Key::Up => self.bar.picker = None,
                    Key::Down | Key::Tab => {}
                    // Another key: the choices put away, as they were, and
                    // the key is the camera's.
                    _ => {
                        self.bar.picker = None;
                        return None;
                    }
                }
                Some(Nav::Stay)
            }
            Picker::Pro { item, slot } => {
                let n = PRO_ITEMS.len();
                match key {
                    Key::Left | Key::Right => {
                        let by = if key == Key::Right { 1 } else { -1 };
                        if PRO_ITEMS[item] == ProItem::Presets {
                            let to = (slot as i32 + by).clamp(0, 2) as usize;
                            self.bar.picker = Some(Picker::Pro { item, slot: to });
                        } else {
                            self.pro_adjust(PRO_ITEMS[item], by);
                        }
                    }
                    Key::Down | Key::Tab => self.bar.picker = Some(Picker::Pro { item: (item + 1) % n, slot }),
                    Key::Up => self.bar.picker = Some(Picker::Pro { item: (item + n - 1) % n, slot }),
                    Key::Enter if PRO_ITEMS[item] == ProItem::Presets => self.recall_preset(slot),
                    Key::Enter | Key::Back => {
                        self.bar.picker = None;
                        self.set_dial(None);
                    }
                    Key::Char('a') => {
                        if let Some(dial) = PRO_ITEMS[item].dial() {
                            let was = self.dial;
                            self.dial = Some(dial);
                            self.dial_auto();
                            self.dial = was;
                        }
                    }
                    // Pro mode's own keys go on to the camera, the row
                    // turning to what each is for.
                    Key::Char(c) => {
                        if let Some(to) = ProItem::of_key(c) {
                            let to = PRO_ITEMS.iter().position(|i| *i == to).unwrap_or(item);
                            let slot = match c {
                                '1'..='3' => c as usize - '1' as usize,
                                _ => slot,
                            };
                            self.bar.picker = Some(Picker::Pro { item: to, slot });
                            // The dials' own keys only turn the row there.
                            if PRO_ITEMS[to].dial().is_some() {
                                return Some(Nav::Stay);
                            }
                        }
                        return None;
                    }
                    _ => return None,
                }
                Some(Nav::Stay)
            }
        }
    }

    /// The words of a chip's choices.
    pub(super) fn choices(&self, chip: Chip) -> Vec<String> {
        match chip {
            Chip::Mode => MODES.iter().map(|(_, name)| (*name).to_owned()).collect(),
            Chip::Timer => camkit::TIMERS.iter().map(|t| if *t == 0 { "off".to_owned() } else { format!("{t} s") }).collect(),
            Chip::Grid => ["off", "thirds", "squares"].map(str::to_owned).to_vec(),
            Chip::Detail => ["auto", "on", "off"].map(str::to_owned).to_vec(),
            Chip::Night => pro::NIGHT_MODES.iter().map(|m| m.words().to_owned()).collect(),
            Chip::Hdr => ["auto", "off"].map(str::to_owned).to_vec(),
            Chip::Size => pixel::SIZES.iter().map(|s| s.words()).collect(),
            Chip::Palette => pixel::PALETTES.iter().map(|p| p.name().to_owned()).collect(),
            Chip::Dither => pixel::DITHERS.iter().map(|d| d.name().to_owned()).collect(),
            Chip::Tone => pixel::CONTRASTS.iter().map(|c| c.name().to_owned()).collect(),
            _ => Vec::new(),
        }
    }

    /// Which of a chip's choices is in use.
    pub(super) fn chosen(&self, chip: Chip, detail: DetailMode) -> usize {
        match chip {
            Chip::Mode => self.mode.index(),
            Chip::Timer => camkit::TIMERS.iter().position(|t| *t == self.timer).unwrap_or(0),
            Chip::Grid => match self.kit.grid {
                Grid::Off => 0,
                Grid::Thirds => 1,
                Grid::Square => 2,
            },
            Chip::Detail => match detail {
                DetailMode::Auto => 0,
                DetailMode::On => 1,
                DetailMode::Off => 2,
            },
            Chip::Size => pixel::SIZES.iter().position(|s| *s == self.pixel.size).unwrap_or(0),
            Chip::Palette => pixel::PALETTES.iter().position(|p| *p == self.pixel.palette).unwrap_or(0),
            Chip::Dither => pixel::DITHERS.iter().position(|d| *d == self.pixel.dither).unwrap_or(0),
            Chip::Tone => pixel::CONTRASTS.iter().position(|c| *c == self.pixel.contrast).unwrap_or(0),
            Chip::Night => pro::NIGHT_MODES.iter().position(|m| *m == self.night_mode).unwrap_or(0),
            Chip::Hdr => usize::from(self.hdr_mode == pro::HdrMode::Off),
            _ => 0,
        }
    }

    fn apply_choice(&mut self, chip: Chip, at: usize, cx: &mut Cx) {
        match chip {
            Chip::Mode => self.set_mode(MODES[at.min(MODES.len() - 1)].0),
            Chip::Timer => self.set_timer(camkit::TIMERS[at.min(camkit::TIMERS.len() - 1)]),
            Chip::Grid => self.set_grid([Grid::Off, Grid::Thirds, Grid::Square][at.min(2)]),
            Chip::Detail => self.set_detail([DetailMode::Auto, DetailMode::On, DetailMode::Off][at.min(2)], cx),
            Chip::Night => self.set_night(pro::NIGHT_MODES[at.min(pro::NIGHT_MODES.len() - 1)]),
            Chip::Hdr => self.set_hdr(if at == 0 { pro::HdrMode::Auto } else { pro::HdrMode::Off }),
            _ => {}
        }
        // The keys stay on the chip, which may have gone with the mode.
        if !self.chips().contains(&chip) {
            self.bar.focus = Some(Chip::Mode);
        }
    }

    pub(super) fn set_timer(&mut self, seconds: u32) {
        self.timer = seconds;
        self.say(if self.timer == 0 { "self-timer off".to_owned() } else { format!("self-timer {} s", self.timer) });
    }

    pub(super) fn set_grid(&mut self, grid: Grid) {
        self.kit.grid = grid;
        self.keep_prefs();
        self.say(match self.kit.grid {
            Grid::Off => "grid off",
            Grid::Thirds => "grid: thirds",
            Grid::Square => "grid: squares",
        });
    }

    pub(super) fn set_detail(&mut self, detail: DetailMode, cx: &mut Cx) {
        cx.settings.detail = detail;
        crate::save_settings(cx.settings);
        self.say(match detail {
            DetailMode::Auto => "detail: auto · merged from a burst in dim light",
            DetailMode::On => "detail: on · every photo merged from a burst",
            DetailMode::Off => "detail: off · one frame a photo",
        });
    }

    /// Exposure's aids, in turn: zebras, false colour, neither (`by` back).
    pub(super) fn step_aids(&mut self, by: i32) {
        let now = match (self.zebras, self.kit.false_colour) {
            (true, _) => 1,
            (false, true) => 2,
            (false, false) => 0,
        };
        let to = (now + by).rem_euclid(3);
        (self.zebras, self.kit.false_colour) = [(false, false), (true, false), (false, true)][to as usize];
        self.tell_develop();
        self.keep_prefs();
        self.say(match to {
            1 => "zebras: highlights at 98%",
            2 => "false colour",
            _ => "exposure aids off",
        });
    }

    fn recall_preset(&mut self, slot: usize) {
        if let Some(setup) = self.kit.presets[slot].clone() {
            self.recall(&setup);
            self.say(format!("c{} recalled", slot + 1));
        } else {
            self.say(format!("c{} is empty · shift {} keeps it", slot + 1, slot + 1));
        }
    }

    /// One of pro's controls turned a step (← is -1).
    fn pro_adjust(&mut self, item: ProItem, by: i32) {
        match item {
            ProItem::Shutter | ProItem::Iso | ProItem::Ev | ProItem::Wb => {
                let dial = item.dial().expect("a dial");
                if dial == Dial::Ev && self.pro.manual() {
                    return self.say("ev shows the meter here");
                }
                let was = self.dial;
                self.dial = Some(dial);
                self.turn_dial(by);
                self.dial = was;
            }
            ProItem::Raw => {
                self.raw = !self.raw;
                self.keep_prefs();
                self.say(if self.raw { "raw on: a .dng too" } else { "RAW off" });
            }
            ProItem::Zebra => self.step_aids(by),
            ProItem::Histogram => {
                self.kit.hist = if by > 0 { self.kit.hist.next() } else { self.kit.hist.next().next() };
                self.tell_develop();
                self.keep_prefs();
            }
            ProItem::Bracket => {
                let at = camkit::BRACKETS.iter().position(|b| *b == self.kit.bracket).unwrap_or(0) as i32;
                self.kit.bracket = camkit::BRACKETS[(at + by).clamp(0, camkit::BRACKETS.len() as i32 - 1) as usize];
                self.keep_prefs();
            }
            ProItem::Priority => {
                let at = camkit::PROGRAMS.iter().position(|p| *p == Program::of(&self.pro)).unwrap_or(0) as i32;
                let program = camkit::PROGRAMS[(at + by).clamp(0, camkit::PROGRAMS.len() as i32 - 1) as usize];
                let still = self.still_now();
                program.apply(&mut self.pro, still.as_ref(), self.metered.as_ref());
                self.pro_changed();
            }
            ProItem::Presets => {}
        }
    }

    /// The dials' values as they read: shutter, ISO, compensation (the
    /// meter, fully manual), white balance; each with whether it is set by
    /// hand (else the camera's own choice).
    pub(super) fn dial_words(&self) -> [(String, bool); 4] {
        let metered = self.metered.unwrap_or_default();
        let still = self.still_now();
        let shutter_us = self.pro.shutter.map_or_else(|| still.map_or(metered.exposure_us, |s| s.exposure_us), |i| pro::SHUTTERS[i].us);
        let iso = self.pro.iso.map_or_else(|| still.map_or(metered.iso(), |s| (s.gain * 100.0).round() as u32), |i| pro::ISOS[i]);
        let wb = match self.pro.wb {
            pro::Wb::Auto => "awb".to_owned(),
            k => k.words(),
        };
        let ev = if self.pro.manual() {
            let stops = pro::meter(&metered, shutter_us, iso as f32 / 100.0);
            pro::ev_words((stops * 3.0).round().clamp(-9.0, 9.0) as i8)
        } else {
            self.pro.ev_words()
        };
        [
            (pro::shutter_words(shutter_us), self.pro.shutter.is_some()),
            (format!("iso {}", pro::iso_words(iso).trim_start_matches("ISO ")), self.pro.iso.is_some()),
            (ev, self.pro.ev != 0 && !self.pro.manual()),
            (wb, self.pro.wb != pro::Wb::Auto),
        ]
    }

    /// What one of pro's controls reads, and whether it is on (or set by hand).
    fn pro_words(&self, item: ProItem, slot: usize) -> (String, bool) {
        match item {
            ProItem::Shutter | ProItem::Iso | ProItem::Ev | ProItem::Wb => {
                let at = PRO_ITEMS.iter().position(|i| *i == item).unwrap_or(0);
                let (words, set) = self.dial_words()[at].clone();
                // Under its name, the ISO's number alone.
                (words.trim_start_matches("iso ").to_owned(), set)
            }
            ProItem::Raw => ((if self.raw { "on" } else { "off" }).to_owned(), self.raw),
            ProItem::Zebra => match (self.zebras, self.kit.false_colour) {
                (true, _) => ("zebras".to_owned(), true),
                (_, true) => ("false colour".to_owned(), true),
                _ => ("off".to_owned(), false),
            },
            ProItem::Histogram => match self.kit.hist {
                HistMode::Luma => ("brightness".to_owned(), true),
                HistMode::Rgb => ("rgb".to_owned(), true),
                HistMode::Off => ("off".to_owned(), false),
            },
            ProItem::Bracket => self.kit.bracket.map_or_else(|| ("off".to_owned(), false), |b| (format!("{} × ±{}", b.frames, b.stops), true)),
            ProItem::Priority => (
                match Program::of(&self.pro) {
                    Program::Auto => "auto",
                    Program::Shutter => "shutter",
                    Program::Iso => "iso",
                    Program::Manual => "manual",
                }
                .to_owned(),
                Program::of(&self.pro) != Program::Auto,
            ),
            ProItem::Presets => {
                let kept = self.kit.presets[slot].is_some();
                (if kept { format!("c{}", slot + 1) } else { format!("c{} · empty", slot + 1) }, kept)
            }
        }
    }

    /// What a chip says it is set to, when it has the keys.
    pub(super) fn chip_words(&self, chip: Chip, detail: DetailMode) -> String {
        match chip {
            Chip::Look => match self.look.texture().words(self.texture) {
                Some(t) => format!("{} · {t}", self.look.name().to_lowercase()),
                None => self.look.name().to_lowercase(),
            },
            Chip::Size => self.pixel.size.words(),
            Chip::Palette => format!("{} · {}", self.pixel.palette.name(), self.pixel.palette.words()),
            Chip::Dither => self.pixel.dither.name().to_owned(),
            Chip::Tone => format!("{} · {}", pixel_mode::bright_words(self.pixel.bright), self.pixel.contrast.name()),
            Chip::Pro => Program::of(&self.pro).words().to_owned(),
            Chip::Lens => (if self.lens == Lens::Main { "this camera" } else { "the smart camera's AI view" }).to_owned(),
            Chip::Night => self.night_words(),
            Chip::Hdr => match (self.hdr_mode, self.hdr_shot()) {
                (pro::HdrMode::Off, _) => "off",
                (_, true) => "auto · on now",
                (_, false) => "auto",
            }
            .to_owned(),
            Chip::Lock => match self.lock {
                pro::Lock::Off => "off · enter or e: lock",
                pro::Lock::Reading(_) => "reading the middle",
                pro::Lock::Held { .. } => "held · e: let go",
            }
            .to_owned(),
            _ => self.choices(chip).get(self.chosen(chip, detail)).cloned().unwrap_or_default(),
        }
    }

    /// Whether a chip's thing is in play now (its icon filled): Night in
    /// the dark, HDR on a backlit scene, the lock held.
    fn chip_live(&self, chip: Chip) -> bool {
        match chip {
            Chip::Night => self.night_on(),
            Chip::Hdr => self.hdr_shot(),
            Chip::Lock => matches!(self.lock, pro::Lock::Held { .. }),
            _ => false,
        }
    }

    /// Whether a chip's setting is on (its icon filled, in ink).
    fn chip_on(&self, chip: Chip, detail: DetailMode) -> bool {
        match chip {
            Chip::Mode | Chip::Lens => true,
            Chip::Night => self.night_mode != pro::NightMode::Off,
            Chip::Hdr => self.hdr_mode == pro::HdrMode::Auto,
            Chip::Lock => self.lock.on(),
            Chip::Look => self.look != Look::Original,
            Chip::Timer => self.timer > 0,
            Chip::Grid => self.kit.grid != Grid::Off,
            Chip::Detail => detail != DetailMode::Off,
            Chip::Pro => self.mode == Mode::Pro,
            Chip::Size | Chip::Palette | Chip::Dither => true,
            Chip::Tone => self.pixel.bright != 0 || self.pixel.contrast != pixel::Contrast::Normal,
        }
    }

    fn chip_icon(&self, chip: Chip) -> &'static str {
        match chip {
            Chip::Mode => match self.mode {
                Mode::Scan => "qr_code_scanner",
                Mode::Doc => "document_scanner",
                Mode::Photo | Mode::Pro => "photo_camera",
                // Drawn in pixels of its own (pixel_mode.rs).
                Mode::Pixel => "",
            },
            Chip::Tone => "sunny",
            Chip::Size | Chip::Palette | Chip::Dither => "",
            Chip::Look => "photo_filter",
            Chip::Timer => "timer",
            Chip::Grid => "grid_on",
            Chip::Detail => "auto_awesome",
            Chip::Pro => "tune",
            Chip::Lens => "",
            Chip::Night => "bedtime",
            // Drawn in letters (draw_chips).
            Chip::Hdr => "",
            Chip::Lock => {
                if self.lock.on() {
                    "lock"
                } else {
                    "lock_open"
                }
            }
        }
    }

    /// A chip's words beside its icon: Night's length while it is in play,
    /// written close as a camera's dial has it ("1s", "2.5s").
    fn chip_face_words(&self, chip: Chip) -> Option<String> {
        match chip {
            Chip::Night => self.night_length().map(|l| l.replace(' ', "")),
            _ => None,
        }
    }

    /// The bar along the viewfinder's foot (or what a chip opened in its
    /// place). Gives how much of the foot it takes, its margin included.
    pub(super) fn draw_bar(&mut self, g: &mut Gfx, vf: Rect) -> f32 {
        // Out of the way while the looks' strip, the intervalometer, the
        // timer, a code found or the alignment have the foot.
        let away = self.strip || self.kit.panel.is_some() || self.kit.run.is_some() || self.countdown.is_some() || self.aligning || self.offered().is_some();
        let want = if self.bar.hidden || away { 0.0 } else { 1.0 };
        if self.bar.shown.target() != want {
            self.bar.shown.set(want);
            g.started_moving();
        }
        let on = if self.bar.has_keys() { 1.0 } else { 0.0 };
        if self.bar.on.target() != on {
            self.bar.on.set(on);
            g.started_moving();
        }
        // Held as set (the shots): at rest at once, as everything else is.
        if g.calm || !self.bar.placed || self.frozen {
            self.bar.shown.jump(self.bar.shown.target());
            self.bar.on.jump(self.bar.on.target());
            self.bar.placed = true;
        }
        let shown = self.bar.shown.value().clamp(0.0, 1.0);
        if shown <= 0.01 {
            return 0.0;
        }
        match self.bar.picker {
            Some(Picker::Pro { item, slot }) => self.draw_pro_row(g, vf, shown, item, slot),
            Some(Picker::Choice { chip, at }) if chip.pixel() => self.draw_pixel_picker(g, vf, shown, chip, at),
            Some(Picker::Choice { chip, at }) => self.draw_choices(g, vf, shown, chip, at),
            None => self.draw_chips(g, vf, shown),
        }
    }

    /// The bar's glass: across the viewfinder `m` in, `bh` tall, sunk out
    /// of sight as it goes.
    pub(super) fn track(g: &mut Gfx, vf: Rect, bh: f32, shown: f32) -> Rect {
        let m = 6.0;
        let r = Rect::new(vf.x + m, (vf.bottom() - m - bh + (1.0 - shown) * (bh + m)).round(), vf.w - 2.0 * m, bh);
        // Its own group: it never melts into the dials' glass above it.
        g.glass(&[bezel::GlassShape { strength: shown, ..bezel::GlassShape::new(r.offset(g.ox, 0.0), bh * 0.5, 13) }]);
        r
    }

    /// The capsule of ice glides to `target` (a cell's place along the bar).
    pub(super) fn glide(&mut self, g: &mut Gfx, target: f32) -> f32 {
        if g.calm || self.frozen || self.bar.on.value() < 0.02 {
            self.bar.at.jump(target);
        } else if self.bar.at.target() != target {
            self.bar.at.set(target);
            g.started_moving();
        }
        self.bar.at.value()
    }

    /// A note above the bar: what the chip in hand is, or what the keys do.
    /// With `key`, the key that acts on it, on a quiet cap at its end.
    pub(super) fn bar_label(g: &mut Gfx, vf: Rect, cx: f32, bottom: f32, words: &str, key: Option<&str>, alpha: f32) -> f32 {
        let cap = g.ty.caption;
        let h = cap.size + 8.0;
        let key_w = key.map_or(0.0, |k| g.measure(k, cap) + 8.0 + 6.0);
        let words = g.text.fit(words, &cap, vf.w - 28.0 - key_w);
        let w = g.measure(&words, cap) + 16.0 + key_w;
        let x = (cx - w * 0.5).clamp(vf.x + 6.0, vf.right() - 6.0 - w);
        let r = Rect::new(x.round(), (bottom - h).round(), w.round(), h);
        g.canvas.rrect(r.offset(g.ox, 0.0), h * 0.5, Color(0, 0, 0), 0.55 * alpha);
        g.text_alpha(r.x + 8.0, r.cy(), &words, cap, Color(255, 255, 255), alpha);
        if let Some(k) = key {
            let kw = key_w - 6.0;
            let c = Rect::new(r.right() - 3.0 - kw, r.y + 3.0, kw, h - 6.0);
            g.canvas.rrect(c.offset(g.ox, 0.0), (c.h * 0.5).min(4.0), Color(255, 255, 255), 0.22 * alpha);
            g.text_alpha(c.x + 4.0, r.cy(), k, cap, Color(255, 255, 255), alpha);
        }
        h + 4.0
    }

    fn draw_chips(&mut self, g: &mut Gfx, vf: Rect, shown: f32) -> f32 {
        let t = g.tokens;
        let cap = g.ty.caption;
        let chips = self.chips();
        let bh = (cap.size + 12.0).round();
        let r = Self::track(g, vf, bh, shown);
        let pad = 3.0;
        let icon = cap.size + 2.0;
        // Each chip as wide as it needs: its icon (the lens's words), with
        // Night's length beside it while it is in play, and its key.
        let words: Vec<Option<String>> = chips.iter().map(|c| self.chip_face_words(*c)).collect();
        let faces: Vec<f32> = chips
            .iter()
            .zip(&words)
            .map(|(c, w)| {
                let base = match c {
                    Chip::Lens => g.measure("AI", cap),
                    // HDR in letters, as the lens's "AI" (its symbol's
                    // letters are too small to read at the bar's size).
                    Chip::Hdr => g.measure("HDR", cap) + 4.0,
                    _ => icon,
                };
                base + w.as_ref().map_or(0.0, |w| 2.0 + g.measure(w, cap))
            })
            .collect();
        // The key a little further off after words, so "1s" and "n" read apart.
        let gaps: Vec<f32> = words.iter().map(|w| if w.is_some() { 6.0 } else { 3.0 }).collect();
        let natural = |g: &mut Gfx, i: usize, badge: bool| faces[i] + if badge { gaps[i] + g.measure(chips[i].badge(), cap) } else { 0.0 } + 10.0;
        let room = r.w - 2.0 * pad;
        let mut badges: Vec<bool> = vec![true; chips.len()];
        let mut widths: Vec<f32> = (0..chips.len()).map(|i| natural(g, i, true)).collect();
        // Too many for the room: the mode's arrows go first (the rail shows
        // the modes); then the row slides along under its glass, every chip
        // keeping its key (1.3.1: with Night, HDR and the lock there are
        // nine; until then every key went).
        if widths.iter().sum::<f32>() > room {
            if let Some(i) = chips.iter().position(|c| *c == Chip::Mode) {
                badges[i] = false;
                widths[i] = natural(g, i, false);
            }
        }
        let total: f32 = widths.iter().sum();
        let sliding = total > room;
        let extra = if sliding { 0.0 } else { ((room - total) / chips.len() as f32).max(0.0) };
        let mut xs = Vec::with_capacity(chips.len());
        let mut run = 0.0;
        for w in &widths {
            xs.push(run);
            run += w + extra;
        }
        // Where the row rests: its start; with the keys, as far along as
        // keeps the chip in hand whole in sight, a chevron's room each end.
        let focus = self.bar.focus.and_then(|f| chips.iter().position(|c| *c == f));
        let edge = if sliding { 9.0 } else { 0.0 };
        let view = room - 2.0 * edge;
        let want = match (sliding, focus) {
            (true, Some(i)) => {
                let mut s = self.bar.slide.target();
                s = s.min(xs[i]).max(xs[i] + widths[i] - view);
                s.clamp(0.0, (total - view).max(0.0))
            }
            _ => 0.0,
        };
        if g.calm || self.frozen || !self.bar.placed {
            self.bar.slide.jump(want);
        } else if self.bar.slide.target() != want {
            self.bar.slide.set(want);
            g.started_moving();
        }
        let slide = self.bar.slide.value();
        let start = r.x + pad + edge - slide;
        let cells: Vec<Rect> = xs.iter().zip(&widths).map(|(x, w)| Rect::new(start + x, r.y + pad, w + extra, bh - 2.0 * pad)).collect();
        let (inner_l, inner_r) = (r.x + pad + edge, r.right() - pad - edge);
        if sliding {
            g.canvas.set_clip(Rect::new(inner_l - 2.0, r.y, inner_r - inner_l + 4.0, r.h).offset(g.ox, 0.0));
        }
        // The capsule, on the chip with the keys.
        let detail = g.detail;
        let on = self.bar.on.value().clamp(0.0, 1.0) * shown;
        let at = self.glide(g, focus.unwrap_or(0) as f32);
        let lerp_cell = |at: f32| {
            let i = (at.floor().max(0.0) as usize).min(cells.len() - 1);
            let j = (i + 1).min(cells.len() - 1);
            let k = (at - i as f32).clamp(0.0, 1.0);
            let (a, b) = (cells[i], cells[j]);
            Rect::new(a.x + (b.x - a.x) * k, a.y, a.w + (b.w - a.w) * k, a.h)
        };
        if on > 0.01 && focus.is_some() {
            let c = lerp_cell(at.clamp(0.0, (cells.len() - 1) as f32));
            g.canvas.rrect(c.offset(g.ox, 0.0), c.h * 0.5, t.ice, on);
        }
        for (i, chip) in chips.iter().enumerate() {
            let c = cells[i];
            // Half out of sight past an end: fading as it goes under the chevron.
            let seen = ((c.right().min(inner_r) - c.x.max(inner_l)) / c.w.max(1.0)).clamp(0.0, 1.0);
            if seen <= 0.0 {
                continue;
            }
            let alpha = shown * if seen < 1.0 { (seen * seen).min(1.0) * 0.6 } else { 1.0 };
            let lit = if focus == Some(i) { on } else { 0.0 };
            let set = self.chip_on(*chip, detail);
            let ink = mix(if set { t.ink } else { t.dim }, t.on_ice, lit);
            let badge_ink = mix(t.dim, t.on_ice, lit);
            let face = faces[i];
            let content = face + if badges[i] { gaps[i] + g.measure(chip.badge(), cap) } else { 0.0 };
            let left = c.cx() - content * 0.5;
            // Night, HDR and the lock fill their icon while in play.
            let filled = if matches!(chip, Chip::Night | Chip::Hdr | Chip::Lock) { self.chip_live(*chip) } else { set };
            if *chip == Chip::Lens {
                g.text_alpha(left, c.cy(), if self.lens == Lens::Main { "1×" } else { "AI" }, cap, ink, alpha);
            } else if *chip == Chip::Hdr {
                // In play, its letters cut out of a plate of ink (as a filled
                // icon is); else in ink, or dim when off.
                if filled {
                    let plate = Rect::new(left, c.cy() - cap.size * 0.5 - 1.0, face, cap.size + 2.0);
                    g.canvas.rrect(plate.offset(g.ox, 0.0), 3.0, ink, alpha);
                    g.text_alpha(left + 2.0, c.cy(), "HDR", cap, mix(t.ground, t.ice, lit), alpha);
                } else {
                    g.text_alpha(left + 2.0, c.cy(), "HDR", cap, ink, alpha);
                }
            } else if self.chip_icon(*chip).is_empty() {
                // Pixel mode's faces, drawn in pixels of their own.
                self.draw_pixel_face(g, *chip, left, c.cy(), face, ink, alpha);
            } else {
                g.icon_alpha(self.chip_icon(*chip), filled, left + icon * 0.5, c.cy(), icon, ink, alpha);
                if let Some(w) = &words[i] {
                    g.text_alpha(left + icon + 2.0, c.cy(), w, cap, ink, alpha);
                }
            }
            if badges[i] {
                g.text_alpha(left + face + gaps[i], c.cy(), chip.badge(), cap, badge_ink, alpha);
            }
        }
        if sliding {
            g.canvas.clear_clip();
            let far = (total - view).max(0.0);
            for (more, x, chevron) in [(slide > 0.5, r.x + pad + 4.0, "‹"), (slide < far - 0.5, r.right() - pad - 4.0, "›")] {
                if more {
                    let half = g.measure(chevron, cap) * 0.5;
                    g.text_alpha(x - half, r.cy(), chevron, cap, t.dim, shown);
                }
            }
        }
        let mut taken = bh + 6.0;
        // With the keys: what the chip is and holds, above it.
        if on > 0.01 {
            if let Some(i) = focus {
                // Whole, with Enter's cap where there is room; else without
                // it; else in fewer words.
                let chip = chips[i];
                let full = format!("{} · {}", chip.name(), self.chip_words(chip, detail));
                let short = match chip {
                    Chip::Pro => format!("pro · {}", self.pro_words(ProItem::Priority, 0).0),
                    Chip::Lens => format!("lens · {}", if self.lens == Lens::Main { "1×" } else { "AI" }),
                    // A film look's grain said only where there is room.
                    Chip::Look => format!("look · {}", self.look.name().to_lowercase()),
                    _ => full.clone(),
                };
                let (room, key_w) = (vf.w - 28.0, g.measure("enter", cap) + 14.0);
                let (words, key) = if g.measure(&full, cap) + key_w <= room {
                    (full, Some("enter"))
                } else if g.measure(&full, cap) <= room {
                    (full, None)
                } else {
                    (short, None)
                };
                taken += on * Self::bar_label(g, vf, cells[i].cx(), r.y - 4.0, &words, key, on);
            }
        }
        taken * shown
    }

    fn draw_choices(&mut self, g: &mut Gfx, vf: Rect, shown: f32, chip: Chip, at: usize) -> f32 {
        let t = g.tokens;
        let cap = g.ty.caption;
        let words = self.choices(chip);
        let bh = (cap.size + 12.0).round();
        let r = Self::track(g, vf, bh, shown);
        let pad = 3.0;
        let cell = (r.w - 2.0 * pad) / words.len().max(1) as f32;
        let slide = self.glide(g, at as f32);
        g.canvas.rrect(Rect::new(r.x + pad + cell * slide + g.ox, r.y + pad, cell, bh - 2.0 * pad), (bh - 2.0 * pad) * 0.5, t.ice, shown);
        let now = self.chosen(chip, g.detail);
        for (i, w) in words.iter().enumerate() {
            let on = (1.0 - (slide - i as f32).abs()).clamp(0.0, 1.0);
            let rest = if i == now { t.ink } else { t.dim };
            let w = g.text.fit(w, &cap, cell - 6.0);
            let half = g.measure(&w, cap) * 0.5;
            g.text_alpha(r.x + pad + cell * (i as f32 + 0.5) - half, r.cy(), &w, cap, mix(rest, t.on_ice, on), shown);
        }
        let name = chip.name();
        let forms = [format!("{name} · enter: set · esc: as it was"), format!("{name} · enter: set"), name.to_owned()];
        let label = forms.iter().find(|f| g.measure(f, cap) + 16.0 <= vf.w - 28.0).unwrap_or(&forms[2]).clone();
        let label_h = Self::bar_label(g, vf, vf.cx(), r.y - 4.0, &label, None, shown);
        (bh + 6.0 + label_h) * shown
    }

    /// Pro's row: each control named (with its key) over what it reads, the
    /// one in hand on ice, the row sliding to keep it in sight.
    fn draw_pro_row(&mut self, g: &mut Gfx, vf: Rect, shown: f32, item: usize, slot: usize) -> f32 {
        let t = g.tokens;
        let cap = g.ty.caption;
        let bh = (2.0 * cap.size + 14.0).round();
        let r = Self::track(g, vf, bh, shown);
        let pad = 3.0;
        let cells: Vec<(f32, String, bool)> = PRO_ITEMS
            .iter()
            .map(|p| {
                let (words, set) = self.pro_words(*p, slot);
                let head = g.measure(p.name(), cap) + 4.0 + g.measure(p.badge(), cap);
                (head.max(g.measure(&words, cap)) + 16.0, words, set)
            })
            .collect();
        // As many whole controls as fit around the one in hand, one more
        // each way in turn, the run of them centred in the row.
        let room = r.w - 2.0 * pad - 20.0;
        let (mut from, mut to) = (item, item);
        let width = |a: usize, b: usize| cells[a..=b].iter().map(|c| c.0).sum::<f32>();
        loop {
            let right = to + 1 < cells.len() && width(from, to + 1) <= room;
            if right {
                to += 1;
            }
            let left = from > 0 && width(from - 1, to) <= room;
            if left {
                from -= 1;
            }
            if !right && !left {
                break;
            }
        }
        let before: f32 = cells[..from].iter().map(|c| c.0).sum();
        let want = before - (r.w - 2.0 * pad - width(from, to)) * 0.5;
        if g.calm || self.frozen || self.bar.on.value() < 0.02 {
            self.bar.scroll.jump(want);
        } else if self.bar.scroll.target() != want {
            self.bar.scroll.set(want);
            g.started_moving();
        }
        let scroll = self.bar.scroll.value();
        let slide = self.glide(g, item as f32);
        let x_of = |at: usize| r.x + pad - scroll + cells[..at].iter().map(|c| c.0).sum::<f32>();
        let i0 = (slide.floor().max(0.0) as usize).min(cells.len() - 1);
        let i1 = (i0 + 1).min(cells.len() - 1);
        let k = (slide - i0 as f32).clamp(0.0, 1.0);
        let (cx0, cw) = (x_of(i0) + (x_of(i1) - x_of(i0)) * k, cells[i0].0 + (cells[i1].0 - cells[i0].0) * k);
        g.canvas.set_clip(r.inset(1.0).offset(g.ox, 0.0));
        let inner = bh - 2.0 * pad;
        g.canvas.rrect(Rect::new(cx0 + g.ox, r.y + pad, cw, inner), (inner * 0.5).min(12.0), t.ice, shown);
        let (name_cy, value_cy) = (r.y + pad + 3.0 + cap.size * 0.5, r.bottom() - pad - 3.0 - cap.size * 0.5);
        // Only whole controls: where more lie past an end, a chevron says so.
        let (inner_l, inner_r) = (r.x + pad + 8.0, r.right() - pad - 8.0);
        let (mut more_l, mut more_r) = (false, false);
        for (i, (p, (w, words, set))) in PRO_ITEMS.iter().zip(&cells).enumerate() {
            let x = x_of(i);
            if x < inner_l - 9.0 {
                more_l = true;
                continue;
            }
            if x + w > inner_r + 9.0 {
                more_r = true;
                continue;
            }
            let on = (1.0 - (slide - i as f32).abs()).clamp(0.0, 1.0);
            let name_w = g.measure(p.name(), cap);
            let head = name_w + 4.0 + g.measure(p.badge(), cap);
            let left = x + (w - head) * 0.5;
            g.text_alpha(left, name_cy, p.name(), cap, mix(t.dim, t.on_ice, on), shown);
            g.text_alpha(left + name_w + 4.0, name_cy, p.badge(), cap, mix(t.faint, t.on_ice, on), shown);
            let vw = g.measure(words, cap);
            g.text_alpha(x + (w - vw) * 0.5, value_cy, words, cap, mix(if *set { t.ink } else { t.dim }, t.on_ice, on), shown);
        }
        for (more, x, chevron) in [(more_l, r.x + pad + 5.0, "‹"), (more_r, r.right() - pad - 5.0, "›")] {
            if more {
                let half = g.measure(chevron, cap) * 0.5;
                g.text_alpha(x - half, r.cy(), chevron, cap, t.dim, shown);
            }
        }
        g.canvas.clear_clip();
        let hint: &[&str] = if PRO_ITEMS[item] == ProItem::Presets {
            &["← → c1 to c3 · enter: recall · shift 1-3: keep", "← → · enter: recall · shift 1-3: keep", "enter: recall"]
        } else if PRO_ITEMS[item].dial().is_some() {
            &["← → change · a: auto · ↑ ↓ or tab: next · esc", "← → change · a: auto · tab: next", "← → change · tab: next", "← → change"]
        } else {
            &["← → change · ↑ ↓ or tab: next · esc", "← → change · tab: next", "← → change"]
        };
        let label = crate::sheet::first_fitting(g, hint, cap, vf.w - 44.0).to_owned();
        let label_h = Self::bar_label(g, vf, vf.cx(), r.y - 4.0, &label, None, shown);
        (bh + 6.0 + label_h) * shown
    }

    /// The Camera's keys, as it is now, for the sheet `?` brings up.
    pub(super) fn keys_now(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        let mut groups = Vec::new();
        if self.aligning {
            return vec![keys("aligning the cameras", &[("arrows", "Move this picture"), ("+ -", "Size it"), ("r", "Start again"), ("enter", "Keep it"), ("esc", "Leave it as it was")])];
        }
        if self.kit.run.is_some() {
            return vec![keys("intervalometer", &[("enter · esc", "Stop it")])];
        }
        if self.stacking.as_ref().is_some_and(|s| s.live) {
            return vec![keys("taking it", &[("hold still", "Until the ring is full"), ("esc", "Cancel: no photo")])];
        }
        if self.kit.panel.is_some() {
            return vec![keys("intervalometer", &[("← →", "Change the value"), ("↑ ↓", "How often, how many"), ("enter", "Start"), ("esc · v", "Put it away")])];
        }
        match self.bar.picker {
            Some(Picker::Choice { chip: Chip::Tone, .. }) => groups.push(keys("tone", &[("← →", "Contrast"), ("↑ ↓", "Brightness"), ("enter", "Keep it"), ("esc", "Leave it as it was")])),
            Some(Picker::Choice { chip, .. }) if chip.pixel() => groups.push(keys("choices", &[("← →", "Choose, seen at once"), ("enter", "Keep it"), ("esc", "Leave it as it was")])),
            Some(Picker::Choice { .. }) => groups.push(keys("choices", &[("← →", "Choose"), ("enter", "Set it"), ("esc", "Leave it as it was")])),
            Some(Picker::Pro { .. }) => groups.push(keys("pro's row", &[("← →", "Change it"), ("↑ ↓ · tab", "The next control"), ("a", "Back to auto"), ("enter", "Recall a preset"), ("esc", "Close the row")])),
            None if self.strip => {
                groups.push(keys("looks", &[("← →", "Choose a look"), ("enter", "Keep it"), ("esc", "Go back to the last")]));
                if self.look == Look::Tint {
                    groups.push(keys("tint", &[("↑ ↓", "Turn its colour")]));
                }
                if self.look.film() && !self.look.texture().is_none() {
                    groups.push(keys("film", &[("↑ ↓", "Grain: off, light, strong")]));
                }
            }
            None if self.bar.focus.is_some() => groups.push(keys("the bar", &[("← → · tab", "Along the chips"), ("enter", "Open its choices"), ("esc · ↑", "Back to the camera")])),
            None => {}
        }
        if self.focus.is_some() {
            groups.push(keys("focus check", &[("arrows", "Look around"), ("+ · esc", "Put it away")]));
        }
        if self.dial.is_some() {
            groups.push(keys("a dial", &[("← →", "Turn it"), ("↑ ↓", "The next dial"), ("a", "Back to auto"), ("esc", "Put it away")]));
        }
        match self.mode {
            Mode::Scan => groups.push(keys("scan", &[("enter · space", "Open what it read"), ("esc", "Put the card away")])),
            Mode::Doc => groups.push(keys("doc", &[("enter · space", "Take the page now"), ("↓", "The pages taken")])),
            Mode::Photo | Mode::Pro => groups.push(keys("taking photos", &[("space · enter", "Take the photo"), ("esc", "Stop a night or HDR photo"), ("↓", "The album")])),
            Mode::Pixel => groups.extend(self.pixel_keys()),
        }
        groups.push(keys("the camera", &[("← →", "Scan, photo, pro, pixel, doc"), ("tab", "The controls bar")]));
        if self.mode == Mode::Pixel && !self.quick {
            if self.smart.is_some() {
                groups.push(keys("lens", &[("l", "1× or the AI view")]));
            }
            groups.push(keys("more", &[("t", "Self-timer: 2, 10 s"), ("n", "Night")]));
            return groups;
        }
        if self.smart.is_some() && !self.quick {
            groups.push(keys("lens", &[("↑ · l", "1× or the AI view")]));
            if self.lens == Lens::Ai {
                groups.push(keys("ai view", &[("a", "Align the two cameras")]));
            }
        }
        if self.quick {
            groups.push(keys("the bar's keys", &[("n", "Night: auto, off, 1, 2, 4 s"), ("t", "Self-timer"), ("g", "Grid"), ("d", "Detail"), ("h", "Level")]));
            return groups;
        }
        // The automatic exposure's (1.3.1): Night in photo and pro modes,
        // HDR and the lock in photo mode (pro's dials have b and e).
        if self.mode == Mode::Photo {
            groups.push(keys(
                "exposure",
                &[("n", "Night: auto, off, 1, 2, 4 s"), ("b", "HDR: auto, off"), ("e", "Lock the exposure on the middle"), ("e again", "Let it go")],
            ));
        } else {
            groups.push(keys("exposure", &[("n", "Night: auto, off, 1, 2, 4 s")]));
        }
        groups.push(keys(
            "the bar's keys",
            &[("k", "Looks"), ("t", "Self-timer: 2, 10 s"), ("g", "Grid"), ("d", "Detail: auto, on, off"), ("p", "Pro's controls")],
        ));
        groups.push(keys("more", &[("h", "Level"), ("o", "Aspect: 3:2, 16:9, 1:1"), ("+", "Check the focus, 1:1")]));
        if self.mode == Mode::Pro {
            groups.push(keys(
                "pro",
                &[
                    ("s", "Shutter"),
                    ("i", "ISO"),
                    ("e", "Exposure (EV)"),
                    ("w", "White balance"),
                    ("y", "WB from a grey card"),
                    ("r", "RAW too"),
                    ("q", "Zebras, false colour"),
                    ("u", "Histogram"),
                    ("b", "Bracketing"),
                    ("m", "Priority"),
                    ("j", "Timing readout"),
                    ("v", "Intervalometer"),
                    ("1 2 3", "Recall a preset"),
                    ("! @ #", "Keep as c1, c2, c3"),
                ],
            ));
        }
        groups
    }
}

/// The bar, each chip's choices, the looks and pro's row, and the camera's
/// sheet of keys, in both tones at the three text sizes (`--keys-shots`):
/// `(name, picture)` for each, the three sizes dark then light.
#[cfg(not(target_arch = "aarch64"))]
pub fn shots() -> Vec<(String, Vec<bezel::Canvas>)> {
    use crate::gfx::TextSize;
    use crate::shell::Shell;
    crate::smartcam::sim::choose(false);
    let sample = scene(PREVIEW_W, PREVIEW_H, 3.1, false);
    let yuv = rgb_to_yuv420(&sample);
    let mut plain = Picture::default();
    Look::Original.compiled(0.0).develop(&yuv, PREVIEW_W, PREVIEW_H, &mut plain, None);
    let hist = pro::Histogram::of_luma(&yuv[..PREVIEW_W * PREVIEW_H], PREVIEW_W, PREVIEW_H, 2);
    let small = shrink_yuv420(&yuv, PREVIEW_W, PREVIEW_H, THUMB.0, THUMB.1);
    let thumbs: Vec<Picture> = LOOKS
        .iter()
        .map(|l| {
            let mut p = Picture::default();
            l.compiled(32.0).develop(&small, THUMB.0, THUMB.1, &mut p, None);
            p
        })
        .collect();
    let metered = Metered { exposure_us: 16_667, analogue: 2.0, digital: 1.0, gains: [1.82, 1.46], ct: 5200, lux: 320.0, settled: true, fom: 900, zoom: 0.0 };
    // Each: the camera as it is set up, then keys to the shell once it is up.
    type Setup = fn(&mut Camera);
    let scenes: Vec<(&str, Setup, &[Key])> = vec![
        ("01-bar", |_| {}, &[]),
        ("02-bar-look-in-hand", |c| c.bar.focus = Some(Chip::Look), &[]),
        ("03-mode-choices", |c| c.bar.picker = Some(Picker::Choice { chip: Chip::Mode, at: 2 }), &[]),
        (
            "04-looks",
            |c| {
                c.bar.focus = Some(Chip::Look);
                c.look = Look::Dramatic;
                c.strip_at.jump(4.0);
                c.set_strip(true);
            },
            &[],
        ),
        ("05-timer-choices", |c| c.bar.picker = Some(Picker::Choice { chip: Chip::Timer, at: 1 }), &[]),
        (
            "06-grid-choices",
            |c| {
                c.kit.grid = Grid::Thirds;
                c.bar.picker = Some(Picker::Choice { chip: Chip::Grid, at: 2 });
            },
            &[],
        ),
        ("07-detail-choices", |c| c.bar.picker = Some(Picker::Choice { chip: Chip::Detail, at: 1 }), &[]),
        (
            "08-pro-bar",
            |c| {
                c.set_mode(Mode::Pro);
                c.pro = Pro { shutter: Some(pro::nearest_shutter(4_000)), ..Pro::default() };
                c.bar.focus = Some(Chip::Pro);
            },
            &[],
        ),
        (
            "09-pro-row",
            |c| {
                c.set_mode(Mode::Pro);
                c.pro = Pro { shutter: Some(pro::nearest_shutter(4_000)), ..Pro::default() };
                c.bar.focus = Some(Chip::Pro);
                c.bar.picker = Some(Picker::Pro { item: 1, slot: 0 });
            },
            &[],
        ),
        (
            "10-pro-row-bracket",
            |c| {
                c.set_mode(Mode::Pro);
                c.kit.bracket = Some(camkit::Bracket { frames: 3, stops: 1 });
                c.bar.focus = Some(Chip::Pro);
                c.bar.picker = Some(Picker::Pro { item: 7, slot: 0 });
            },
            &[],
        ),
        ("11-shooting-bar-away", |c| c.bar.hidden = true, &[]),
        ("12-camera-keys", |_| {}, &[Key::Char('?')]),
        ("13-camera-keys-pro", |c| c.set_mode(Mode::Pro), &[Key::Char('?'), Key::PageDown, Key::PageDown, Key::PageDown]),
    ];
    let mut out: Vec<(String, Vec<bezel::Canvas>)> = scenes.iter().map(|(n, _, _)| ((*n).to_owned(), Vec::new())).collect();
    for light in [false, true] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            for (n, (_, setup, keys)) in scenes.iter().enumerate() {
                let mut camera = Camera::new();
                camera.frozen = true;
                camera.flash.jump(0.0);
                (camera.look, camera.hue, camera.raw, camera.zebras, camera.pro, camera.timer) = (Look::Original, 32.0, false, false, Pro::default(), 0);
                camera.kit = Kit::default();
                camera.last = None;
                camera.last_rx = None;
                camera.preview = Some(plain.clone());
                camera.thumbs = thumbs.clone();
                camera.histogram = hist.clone();
                camera.metered = Some(metered);
                camera.note = None;
                let mut shell = Shell::new(crate::robot::Robot::demo());
                (shell.settings.text, shell.settings.light) = (size, light);
                shell.apply_settings();
                setup(&mut camera);
                camera.note = None;
                for a in [&mut camera.mode_at, &mut camera.dial_at, &mut camera.dial_fade, &mut camera.strip_at, &mut camera.strip_fade, &mut camera.focus_fade] {
                    a.jump(a.target());
                }
                shell.show(Box::new(camera));
                for _ in 0..60 {
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                }
                for key in keys.iter() {
                    shell.key(*key);
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                }
                for _ in 0..90 {
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                }
                out[n].1.push(shell.gfx.canvas.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Settings;

    fn press(camera: &mut Camera, settings: &mut Settings, key: Key) -> Nav {
        let mut robot = Robot::none();
        let mut say = None;
        camera.key(key, &mut Cx { robot: &mut robot, settings, say: &mut say, at_rest: true })
    }

    fn camera() -> Camera {
        crate::shell::tests::sandbox();
        let mut camera = Camera::new();
        (camera.look, camera.timer, camera.kit.grid) = (Look::Original, 0, Grid::Off);
        (camera.night_mode, camera.hdr_mode) = (crate::pro::NightMode::Auto, crate::pro::HdrMode::Auto);
        camera
    }

    #[test]
    fn tab_walks_the_bar_and_its_choices_set_what_they_say() {
        let _port = crate::smartcam::sim::exclusive(false);
        let (mut camera, mut s) = (camera(), Settings::default());
        assert_eq!(camera.chips(), [Chip::Mode, Chip::Night, Chip::Hdr, Chip::Lock, Chip::Look, Chip::Timer, Chip::Grid, Chip::Detail, Chip::Pro]);
        let mut key = |camera: &mut Camera, k: Key| {
            press(camera, &mut s, k);
        };
        // Tab: the keys into the bar, on the look; → and Tab along it, ←
        // back, never past its ends, and never changing the mode.
        key(&mut camera, Key::Tab);
        assert_eq!(camera.bar.focus, Some(Chip::Look));
        key(&mut camera, Key::Right);
        assert_eq!(camera.bar.focus, Some(Chip::Timer));
        key(&mut camera, Key::Tab);
        assert_eq!(camera.bar.focus, Some(Chip::Grid));
        for _ in 0..8 {
            key(&mut camera, Key::Left);
        }
        assert_eq!((camera.bar.focus, camera.mode), (Some(Chip::Mode), Mode::Photo));
        // Night's choices: auto, off, 1 s, 2 s, 4 s.
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Enter);
        assert_eq!(camera.bar.picker, Some(Picker::Choice { chip: Chip::Night, at: 0 }));
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Enter);
        assert_eq!(camera.night_mode, crate::pro::NightMode::Fixed(1));
        key(&mut camera, Key::Enter);
        key(&mut camera, Key::Left);
        key(&mut camera, Key::Left);
        key(&mut camera, Key::Enter);
        assert_eq!(camera.night_mode, crate::pro::NightMode::Auto);
        // The lock's chip locks with Enter, and lets go.
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Enter);
        assert!(camera.lock.on());
        key(&mut camera, Key::Enter);
        assert!(!camera.lock.on());
        // The timer's choices: off, 2 s, 10 s; Enter sets the one in hand.
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Enter);
        assert_eq!(camera.bar.picker, Some(Picker::Choice { chip: Chip::Timer, at: 0 }));
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Right);
        assert_eq!(camera.timer, 0, "not before Enter");
        key(&mut camera, Key::Enter);
        assert_eq!((camera.timer, camera.bar.picker, camera.bar.focus), (10, None, Some(Chip::Timer)));
        // Esc leaves it as it was.
        key(&mut camera, Key::Enter);
        key(&mut camera, Key::Left);
        key(&mut camera, Key::Back);
        assert_eq!((camera.timer, camera.bar.picker), (10, None));
        // The look: its strip of looks; → chooses, Enter keeps it.
        key(&mut camera, Key::Left);
        key(&mut camera, Key::Enter);
        assert!(camera.strip);
        key(&mut camera, Key::Right);
        assert_eq!(camera.look, Look::Vivid, "shown as it will be saved");
        key(&mut camera, Key::Enter);
        assert!(!camera.strip && camera.look == Look::Vivid && camera.saving.is_none());
        assert_eq!(camera.bar.focus, Some(Chip::Look), "the keys back on its chip");
        // Esc on the strip goes back to the look it opened on.
        key(&mut camera, Key::Enter);
        key(&mut camera, Key::Right);
        assert_eq!(camera.look, Look::VividWarm);
        key(&mut camera, Key::Back);
        assert!(!camera.strip && camera.look == Look::Vivid);
        // The grid, and detail (a setting of the handheld's).
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Enter);
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Enter);
        assert_eq!(camera.kit.grid, Grid::Thirds);
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Enter);
        key(&mut camera, Key::Right);
        key(&mut camera, Key::Enter);
        drop(key);
        assert_eq!(s.detail, DetailMode::On);
        // Esc gives the keys back: ← → are the modes again.
        press(&mut camera, &mut s, Key::Back);
        press(&mut camera, &mut s, Key::Left);
        assert_eq!((camera.bar.focus, camera.mode), (None, Mode::Scan));
    }

    #[test]
    fn pro_turns_each_control_in_its_row_and_the_shutter_sends_the_bar_down() {
        let _port = crate::smartcam::sim::exclusive(false);
        let (mut camera, mut s) = (camera(), Settings::default());
        camera.pro = Pro::default();
        camera.raw = false;
        // p: pro mode, and its row on the shutter.
        press(&mut camera, &mut s, Key::Char('p'));
        assert_eq!((camera.mode, camera.bar.picker), (Mode::Pro, Some(Picker::Pro { item: 0, slot: 0 })));
        press(&mut camera, &mut s, Key::Right);
        assert!(camera.pro.shutter.is_some(), "the shutter set by hand");
        press(&mut camera, &mut s, Key::Char('a'));
        assert!(camera.pro.shutter.is_none(), "a: back to auto");
        // ↓ to the ISO, Tab on to RAW.
        press(&mut camera, &mut s, Key::Down);
        press(&mut camera, &mut s, Key::Right);
        assert!(camera.pro.iso.is_some());
        for _ in 0..3 {
            press(&mut camera, &mut s, Key::Tab);
        }
        assert_eq!(camera.bar.picker, Some(Picker::Pro { item: 4, slot: 0 }));
        press(&mut camera, &mut s, Key::Left);
        assert!(camera.raw);
        // Pro's own letters turn the row to what they are for.
        press(&mut camera, &mut s, Key::Char('b'));
        assert_eq!(camera.bar.picker, Some(Picker::Pro { item: 7, slot: 0 }));
        assert!(camera.kit.bracket.is_some(), "b brackets, as ever");
        // Esc closes the row, the keys on its chip; Esc again gives them back.
        press(&mut camera, &mut s, Key::Back);
        assert_eq!((camera.bar.picker, camera.bar.focus), (None, Some(Chip::Pro)));
        press(&mut camera, &mut s, Key::Back);
        assert_eq!(camera.bar.focus, None);
        // The shutter (here scan's) sends the bar down; another key brings it back.
        camera.set_mode(Mode::Scan);
        press(&mut camera, &mut s, Key::Char(' '));
        assert!(camera.bar.hidden);
        press(&mut camera, &mut s, Key::Char('g'));
        assert!(!camera.bar.hidden);
    }

    #[test]
    fn the_bar_and_every_choice_fit_at_every_text_size() {
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = camera();
        let mut g = Gfx::new(bezel::Tokens::dark());
        let vf = Camera::viewfinder();
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            for picker in [None, Some(Picker::Choice { chip: Chip::Timer, at: 1 }), Some(Picker::Pro { item: 9, slot: 0 })] {
                camera.bar.picker = picker;
                camera.bar.focus = Some(Chip::Detail);
                let taken = camera.draw_bar(&mut g, vf);
                // Clear of the focus point, the picture's middle.
                assert!(taken > 10.0 && vf.bottom() - taken > vf.cy() + 4.0, "{size:?} {picker:?}: {taken}");
            }
        }
        // The keys it lists fit their sheet.
        crate::keysheet::fits(&mut g, &camera.keys_now()).unwrap();
        camera.mode = Mode::Pro;
        crate::keysheet::fits(&mut g, &camera.keys_now()).unwrap();
    }
}
