//! The shell's side of the clipboard (`clipboard/`) and quick capture
//! (`capture.rs`): their keys, their sheets of glass over the screen in
//! front, copying from and pasting into it, and what the island says.
//!
//! The order the keys go in (see `Shell::key`): past the lock and the tour,
//! Ctrl-Shift-H and Ctrl-Shift-N bring their sheets up from anywhere (the
//! recent apps, Spotlight and the control centre give way to them); while
//! one is up it takes the keys, as Spotlight does, giving way itself to
//! Ctrl-Tab, Ctrl-Space and (the history) Ctrl-Enter. Copy and paste come
//! last, just before the screen in front: Ctrl-C and Ctrl-V there, or
//! Ctrl-Shift-C and V, which a terminal (`Screen::keeps_ctrl`) takes itself.

use super::*;
use crate::capture::{Capture, Receipt};
use crate::clipboard::{Hotkey, Panel, Sensitivity, CAPTURE_KEY, HISTORY_KEY};

/// Which sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    Clipboard,
    Capture,
}

/// The sheets' own state in the shell.
pub struct Sheets {
    pub panel: Panel,
    pub capture: Capture,
    /// While one is out: the frame it came up over, and the page dots that
    /// frame asked for (held still behind it, as Spotlight's).
    under: Option<(Vec<u32>, Option<(usize, usize, f32)>)>,
    /// Where the last capture went, while the island says so: Ctrl-Enter
    /// opens it there.
    receipt: Option<(Receipt, f32)>,
}

impl Sheets {
    pub fn new() -> Sheets {
        Sheets { panel: Panel::new(), capture: Capture::new(), under: None, receipt: None }
    }

    pub fn visible(&self) -> bool {
        self.panel.visible() || self.capture.visible()
    }

    pub fn open(&self) -> bool {
        self.panel.is_open() || self.capture.is_open()
    }

    fn gone_now(&mut self) {
        self.panel.gone_now();
        self.capture.gone_now();
        self.under = None;
    }
}

impl Shell {
    /// A key for the sheets, before anything but the lock and the tour:
    /// true if they took it.
    pub(super) fn sheets_key(&mut self, key: Key) -> bool {
        // The backlight and the sound keys work over them, as over Spotlight.
        if matches!(key, Key::BrightnessDown | Key::BrightnessUp | Key::Media(_)) {
            return false;
        }
        // On its way back into the island: its own key turns it round; any
        // other key makes it go at once, and lands on the screen beneath.
        let s = &mut self.sheets;
        if !s.capture.is_open() && s.capture.visible() && key != CAPTURE_KEY {
            s.capture.gone_now();
            s.under = None;
        }
        if !s.panel.is_open() && s.panel.visible() && key != HISTORY_KEY {
            s.panel.gone_now();
            s.under = None;
        }
        // The recent apps and the control centre in their place; Spotlight
        // in the history's (in quick capture, Ctrl-Enter saves and opens).
        let gives_way = matches!(key, Key::Ctrl('\t') | Key::Ctrl(' ')) || (key == Key::Ctrl('\n') && self.sheets.panel.is_open());
        if self.sheets.open() && gives_way {
            self.sheets.gone_now();
            return false;
        }
        if self.sheets.capture.is_open() {
            self.capture_key(key);
            return true;
        }
        if self.sheets.panel.is_open() {
            self.panel_key(key);
            return true;
        }
        match key {
            HISTORY_KEY => self.summon_sheet(Sheet::Clipboard),
            CAPTURE_KEY => self.summon_sheet(Sheet::Capture),
            // Where the last capture went, while the island says it.
            Key::Ctrl('\n') if self.sheets.receipt.is_some() => self.open_receipt(),
            _ => return false,
        }
        true
    }

    /// Bring a sheet up over whatever is showing (never over a lock): what
    /// else is up gives way to it at once.
    pub(crate) fn summon_sheet(&mut self, which: Sheet) {
        if self.locked() || self.intro.as_ref().is_some_and(|i| !i.leaving()) || self.tour.is_some() {
            return;
        }
        if self.switcher.is_some() {
            (self.switcher, self.switcher_under) = (None, None);
        }
        if self.spotlight.visible() {
            self.spotlight.gone_now();
            self.spotlight_under = None;
        }
        if self.control.visible() {
            self.control.set_open(false, &self.settings);
            self.control.shown.jump(0.0);
            self.control_under = None;
            self.control_glass = None;
        }
        // The island no longer says where the last capture went: a sheet
        // comes out of it now.
        if self.sheets.receipt.take().is_some() {
            self.message = None;
        }
        let calm = self.settings.calm;
        let s = &mut self.sheets;
        if !s.visible() {
            s.under = None;
        }
        match which {
            Sheet::Clipboard => {
                s.capture.gone_now();
                s.panel.summon(&self.gfx, calm);
            }
            Sheet::Capture => {
                s.panel.gone_now();
                s.capture.summon(&self.gfx, calm, &self.robot, self.settings.clock24);
            }
        }
        self.moving = true;
        self.dirty = true;
    }

    /// Whether a sheet is up (the tests, the gallery).
    #[allow(dead_code)]
    pub(crate) fn sheet_open(&self) -> Option<Sheet> {
        if self.sheets.capture.is_open() {
            Some(Sheet::Capture)
        } else if self.sheets.panel.is_open() {
            Some(Sheet::Clipboard)
        } else {
            None
        }
    }

    fn capture_key(&mut self, key: Key) {
        use crate::capture::Outcome;
        self.moving = true;
        match self.sheets.capture.key(key, &self.robot, self.settings.clock24) {
            Outcome::Stay | Outcome::Say(_) => {}
            Outcome::Close => self.sheets.capture.dismiss(),
            Outcome::Saved(receipt, false) => {
                self.sheets.capture.dismiss();
                self.show_receipt(receipt);
            }
            Outcome::Saved(receipt, true) => {
                self.sheets.receipt = Some((receipt, 0.0));
                self.open_receipt();
            }
            Outcome::Photo => {
                // Over the camera already: that is where photos are taken.
                if self.top_title() == "Camera" {
                    self.sheets.capture.dismiss();
                    return self.say("the camera is open");
                }
                let from = self.sheets.capture.rect();
                let backdrop = self.gfx.canvas.px.clone();
                self.sheets.gone_now();
                drop(self.interrupt());
                self.open_over(Box::new(crate::camera::Camera::quick()), from, backdrop, false);
                self.apply_settings();
            }
        }
    }

    fn panel_key(&mut self, key: Key) {
        use crate::clipboard::panel::Outcome;
        self.moving = true;
        match self.sheets.panel.key(key) {
            Outcome::Stay | Outcome::Say(_) => {}
            Outcome::Close => self.sheets.panel.dismiss(),
            // Into the field in front, as the panel goes; where nothing takes
            // text, only copied.
            Outcome::Paste(text) => {
                self.sheets.panel.dismiss();
                let takes = self.stack.last().is_some_and(|(top, _)| top.wants_text());
                if takes {
                    crate::clipboard::copy_quietly(&text, Sensitivity::Normal);
                    self.paste_in_front(Some(text));
                } else {
                    crate::clipboard::copy(&text, Sensitivity::Normal);
                }
            }
        }
    }

    /// The island says where a capture went, for a few seconds, and Ctrl-Enter
    /// meanwhile opens it there.
    fn show_receipt(&mut self, receipt: Receipt) {
        self.say_fitting(&receipt.words, crate::capture::RECEIPT_SECONDS);
        self.sheets.receipt = Some((receipt, crate::capture::RECEIPT_SECONDS));
    }

    /// What the last capture went into, opened on it, growing out of the
    /// island over the screen in front (Esc comes back to it).
    fn open_receipt(&mut self) {
        let Some((receipt, _)) = self.sheets.receipt.take() else { return };
        self.message = None;
        match receipt.open() {
            Ok(screen) => {
                let from = self.places.island.unwrap_or_else(|| self.island_rect());
                let backdrop = self.gfx.canvas.px.clone();
                self.sheets.gone_now();
                drop(self.interrupt());
                self.open_over(screen, from, backdrop, false);
                self.apply_settings();
            }
            Err(why) => self.say(why),
        }
        self.moving = true;
        self.dirty = true;
    }

    /// Copy: what the screen in front says it copies, else the field being
    /// typed in, else the row in focus, as they were last drawn. While a
    /// screen takes text, only what is typed: never the row around it (a
    /// key or a password typed into a row of its own). Nothing from a
    /// passcode's page.
    pub(super) fn copy_in_front(&mut self) {
        let at_rest = self.transition.is_none();
        let (top, _) = self.stack.last_mut().expect("the home screen is always there");
        if top.secret() {
            return self.say("nothing is copied from here");
        }
        let typing = top.wants_text();
        let field = self.gfx.copy_field.clone().filter(|t| !t.trim().is_empty());
        let drawn = if !at_rest { None } else if typing { field } else { field.or_else(|| self.gfx.copy_row.clone()) };
        match top.copy(&self.robot).or(drawn).filter(|t| !t.trim().is_empty()) {
            Some(text) => crate::clipboard::copy(&text, Sensitivity::Normal),
            None => self.say("nothing here to copy"),
        }
        self.moving = true;
    }

    /// Paste `text` (else what is on the clipboard) into the screen in
    /// front, where it takes text.
    pub(super) fn paste_in_front(&mut self, text: Option<String>) {
        let Some(text) = text.or_else(crate::clipboard::paste_text) else { return self.say("the clipboard is empty") };
        let (top, _) = self.stack.last_mut().expect("the home screen is always there");
        if !top.wants_text() {
            return self.say("nothing here takes text");
        }
        let mut say = None;
        let at_rest = self.transition.is_none();
        let nav = top.paste(&text, &mut Cx { robot: &mut self.robot, settings: &mut self.settings, say: &mut say, at_rest });
        if let Some(words) = say {
            self.say(words);
        }
        self.follow(nav);
        self.apply_settings();
        self.moving = true;
    }

    /// The first of `words` that fits the island whole, said for `seconds`.
    pub(crate) fn say_fitting(&mut self, words: &[String], seconds: f32) {
        // The island's room for a message, less its dot and padding.
        let room = 246.0;
        let caption = self.gfx.ty.caption;
        let Some(text) = words.iter().find(|w| self.gfx.measure(w, caption) <= room).or(words.last()).cloned() else { return };
        self.say(text);
        if let Some((_, left)) = &mut self.message {
            *left = seconds;
        }
    }

    /// The sheets in time: their springs, the clipboard's secret clearing,
    /// what the island says of the clipboard, and the photo quick capture
    /// took. Under the lock they go at once, and so does a secret.
    pub(super) fn tick_sheets(&mut self, dt: f32, motion_dt: f32) -> bool {
        crate::clipboard::tick();
        let locked = self.locked();
        if locked {
            crate::clipboard::lock();
            if self.sheets.visible() {
                self.sheets.gone_now();
                self.dirty = true;
            }
            self.sheets.receipt = None;
        }
        // Said only to someone looking: a copy from a script, or a secret
        // clearing, with the screen off is no notice to blink the light for.
        if let Some(words) = crate::clipboard::take_words() {
            if !locked && self.standby.state() == crate::standby::State::Awake {
                self.say_fitting(&words, MESSAGE_SECONDS);
            }
        }
        if let Some(receipt) = crate::capture::take_photo() {
            if !locked {
                self.show_receipt(receipt);
            }
        }
        if let Some((_, left)) = &mut self.sheets.receipt {
            *left -= dt;
            if *left <= 0.0 {
                self.sheets.receipt = None;
            }
        }
        let moving = self.sheets.panel.tick(dt, motion_dt) | self.sheets.capture.tick(dt, motion_dt);
        if !self.sheets.visible() && self.sheets.under.is_some() {
            self.sheets.under = None;
            self.dirty = true;
        }
        moving
    }

    /// A sheet over the frame held behind it: the frame dims a little as it
    /// comes, its glass goes through one pass with the island's, and what
    /// sits on it is drawn on top. True if one was drawn.
    pub(super) fn draw_sheets(&mut self) -> bool {
        if !self.sheets.visible() {
            return false;
        }
        if self.transition.is_some() {
            self.sheets.under = None;
        }
        match &self.sheets.under {
            Some((under, pager)) if under.len() == self.gfx.canvas.px.len() => {
                self.gfx.canvas.px.copy_from_slice(under);
                self.gfx.pager = *pager;
            }
            _ => {
                // Drawn without the island's glass: the sheet is drawn with it.
                self.draw_frame();
                if self.transition.is_none() {
                    self.sheets.under = Some((self.gfx.canvas.px.clone(), self.gfx.pager));
                }
            }
        }
        let began = std::time::Instant::now();
        let capture = self.sheets.capture.visible();
        let dim = if capture { self.sheets.capture.dim() } else { self.sheets.panel.dim() };
        let dim = (dim.clamp(0.0, 1.0) * 256.0) as u32;
        if dim > 0 {
            bezel::simd::mix_into(&mut self.gfx.canvas.px, None, dim);
        }
        let island = self.island_rect();
        let (shape, shown) = if capture {
            (self.sheets.capture.glass(&self.gfx, island), self.sheets.capture.shown())
        } else {
            (self.sheets.panel.glass(&self.gfx, island), self.sheets.panel.shown())
        };
        self.draw_chrome(began, Some(shape), shown);
        if capture {
            let talk = self.voice.island();
            self.sheets.capture.draw(&mut self.gfx, island, talk.as_ref());
        } else {
            self.sheets.panel.draw(&mut self.gfx, island);
        }
        if self.gfx.take_motion_started() {
            self.moving = true;
        }
        true
    }

    /// What was said, written down, for quick capture's sheet.
    pub(super) fn heard_for_capture(&mut self, text: &str) {
        if self.sheets.capture.is_open() {
            let said = self.sheets.capture.dictated(text, &self.robot, self.settings.clock24);
            self.say(said);
        } else {
            self.say(format!("heard: {text}"));
        }
        self.moving = true;
        self.dirty = true;
    }

    /// What `key` asks of the clipboard over the screen in front, after
    /// everything else has had its turn: Copy or Paste, done here. The key
    /// the screen should be given otherwise (a Ctrl-Shift-letter the shell
    /// has no use for is the screen's own Ctrl-letter, except in a terminal,
    /// which takes Ctrl-Shift-C and V itself).
    pub(super) fn clipboard_key(&mut self, key: Key) -> Option<Key> {
        let keeps_ctrl = self.stack.last().is_some_and(|(top, _)| top.keeps_ctrl());
        match crate::clipboard::hotkey(key, keeps_ctrl) {
            Some(Hotkey::Copy) => {
                self.copy_in_front();
                None
            }
            Some(Hotkey::Paste) => {
                self.paste_in_front(None);
                None
            }
            // Taken before: a sheet that could not come (a lock) takes nothing past it either.
            Some(Hotkey::History | Hotkey::Capture) => None,
            None => Some(match key {
                Key::Ctrl(c) if c.is_ascii_uppercase() && !keeps_ctrl => Key::Ctrl(c.to_ascii_lowercase()),
                other => other,
            }),
        }
    }

    /// Spotlight's own copy and paste: what it found (or what is typed),
    /// and the clipboard into what is typed.
    pub(super) fn spotlight_clipboard(&mut self, key: Key) -> bool {
        match crate::clipboard::hotkey(key, false) {
            Some(Hotkey::Copy) => {
                let text = self.spotlight.copy_text();
                if text.trim().is_empty() {
                    self.say("nothing here to copy");
                } else {
                    crate::clipboard::copy(&text, Sensitivity::Normal);
                }
                true
            }
            Some(Hotkey::Paste) => {
                match crate::clipboard::paste_text() {
                    Some(text) => self.spotlight.paste(&text, &self.robot),
                    None => self.say("the clipboard is empty"),
                }
                true
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------- gallery ---

/// `--clip-shots <dir>`: the clipboard's history, quick capture, the
/// receipt, the Terminal's copy mode and the shortcut list, each as a sheet
/// of both looks (Catalyst, Bezel: across) at the three text sizes (down);
/// and the sheets' motion, as frames and an animated PNG.
#[cfg(not(target_arch = "aarch64"))]
pub fn gallery(dir: &std::path::Path) {
    use crate::gfx::TextSize;
    use crate::look::Theme;
    // A folder of its own: notes, reminders and the clipboard it makes stay there.
    let own = std::env::temp_dir().join(format!("catalyst-clip-shots-{}", std::process::id()));
    std::fs::remove_dir_all(&own).ok();
    std::fs::create_dir_all(&own).ok();
    std::env::set_var("CATALYST_CONF", own.join("robot"));
    std::env::set_var("CATALYST_FILES", own.join("files"));
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let frames = |shell: &mut Shell, n: usize| {
        for _ in 0..n {
            shell.tick(1.0 / 60.0);
            shell.draw();
        }
    };
    let type_in = |shell: &mut Shell, text: &str| {
        for c in text.chars() {
            shell.key(Key::Char(c));
        }
    };
    // The history as a day at an event leaves it: a pinned login, a link,
    // a note's lines, a value and an address, and a password from the vault.
    let fill = |secret: bool| {
        crate::clipboard::with(|b| {
            b.clear();
            for item in b.items() {
                b.remove(&item.text);
            }
            let (now, unix) = (std::time::Instant::now(), chrono::Utc::now().timestamp());
            b.copy("ssh admin@10.58.5.2", Sensitivity::Normal, now, unix - 3 * 86_400);
            b.toggle_pin("ssh admin@10.58.5.2");
            b.copy("https://frc-events.firstinspires.org/2026/cmptx", Sensitivity::Normal, now, unix - 7200);
            b.copy("Swap the intake belt before Q32,\ncheck the CAN wiring at the PDH,\nand charge battery 3 for the next match.", Sensitivity::Normal, now, unix - 1500);
            b.copy("12.42", Sensitivity::Normal, now, unix - 240);
            b.copy("10.58.5.2", Sensitivity::Normal, now, unix - 20);
            if secret {
                b.copy("correct-horse-battery", Sensitivity::Secret, now, unix);
            }
        });
        crate::clipboard::take_words();
    };
    let printed = "$ ip -4 addr show wlan0\r\n3: wlan0: <UP,LOWER_UP> mtu 1500\r\n    inet 10.58.5.20/24 brd 10.58.5.255\r\n$ ping -c1 10.58.5.2\r\n64 bytes from 10.58.5.2: time=2.1 ms\r\n$ ";
    type Scene = (&'static str, fn(&mut Shell));
    let scenes: Vec<Scene> = vec![
        ("history", |s| s.show(Box::new(crate::notes::Notes::new()))),
        ("history-whole", |s| s.show(Box::new(crate::notes::Notes::new()))),
        ("history-empty", |s| s.show(Box::new(crate::notes::Notes::new()))),
        ("capture", |s| s.show(Box::new(crate::terminal::Terminal::showing("$ ", 40, 8, 0)))),
        ("capture-note", |_| {}),
        ("capture-reminder", |_| {}),
        ("capture-listening", |_| {}),
        ("capture-long", |_| {}),
        ("receipt", |s| s.show(Box::new(crate::screens::RobotScreen::default()))),
        ("terminal-copy", |_| {}),
        ("shortcuts-everywhere", |s| s.show(Box::new(crate::help::Shortcuts::new()))),
        ("shortcuts-sheets", |s| s.show(Box::new(crate::help::Shortcuts::new()))),
    ];
    let (w, h) = (W as usize, H as usize);
    let gap = 4usize;
    for (name, scene) in &scenes {
        let mut sheet = bezel::Canvas::new((2 * w + gap) as i32, (3 * h + 2 * gap) as i32);
        sheet.px.fill(0x202124);
        for (column, theme) in [Theme::Catalyst, Theme::Bezel].into_iter().enumerate() {
            for (row, size) in [TextSize::Compact, TextSize::Standard, TextSize::Large].into_iter().enumerate() {
                fill(!name.ends_with("empty"));
                if name.ends_with("empty") {
                    crate::clipboard::with(|b| {
                        for item in b.items() {
                            b.remove(&item.text);
                        }
                    });
                }
                let mut shell = Shell::new(crate::robot::Robot::demo());
                shell.settings.theme = theme;
                (shell.settings.wallpaper, shell.settings.accent) = theme.defaults();
                shell.settings.text = size;
                shell.apply_settings();
                frames(&mut shell, 100);
                scene(&mut shell);
                frames(&mut shell, 60);
                match *name {
                    "history" | "history-empty" => shell.key(HISTORY_KEY),
                    "history-whole" => {
                        shell.key(HISTORY_KEY);
                        frames(&mut shell, 30);
                        // The secret, the pinned login, an address, a value: then the lines.
                        for _ in 0..4 {
                            shell.key(Key::Down);
                        }
                        shell.key(Key::Tab);
                    }
                    "capture" => shell.key(CAPTURE_KEY),
                    "capture-note" | "capture-reminder" | "capture-listening" | "capture-long" | "receipt" => {
                        if *name != "receipt" {
                            let (cols, rows) = crate::terminal::Terminal::grid_for(&mut shell.gfx);
                            shell.show(Box::new(crate::terminal::Terminal::showing(printed, cols, rows, 0)));
                            frames(&mut shell, 60);
                        }
                        shell.key(CAPTURE_KEY);
                        frames(&mut shell, 20);
                        match *name {
                            "capture-note" => type_in(&mut shell, "Intake belt slips at full speed"),
                            "capture-reminder" => type_in(&mut shell, "swap battery in 10 min"),
                            "capture-listening" => {
                                type_in(&mut shell, "check the bumpers");
                                let island = crate::voice::Island { icon: "mic", hot: true, words: "let go to type".into(), bars: [0.45, 0.75, 1.0, 0.7, 0.4] };
                                shell.stage_island(Some(island), None);
                            }
                            "capture-long" => {
                                crate::clipboard::copy_quietly("Swap the intake belt before Q32,\ncheck the CAN wiring at the PDH,\nand charge battery 3 for the next match.", Sensitivity::Normal);
                                type_in(&mut shell, "Pit:");
                                shell.key(Key::Ctrl('v'));
                            }
                            _ => {
                                type_in(&mut shell, "Bumper bolts loose on the left side");
                                shell.key(Key::Enter);
                            }
                        }
                    }
                    "terminal-copy" => {
                        let (cols, rows) = crate::terminal::Terminal::grid_for(&mut shell.gfx);
                        shell.show(Box::new(crate::terminal::Terminal::showing(printed, cols, rows, 2)));
                    }
                    "shortcuts-everywhere" => {
                        for _ in 0..11 {
                            shell.key(Key::Down);
                        }
                    }
                    "shortcuts-sheets" => {
                        for _ in 0..27 {
                            shell.key(Key::Down);
                        }
                    }
                    _ => {}
                }
                frames(&mut shell, 90);
                shell.draw();
                for y in 0..h {
                    let at = (row * (h + gap) + y) * (2 * w + gap) + column * (w + gap);
                    sheet.px[at..at + w].copy_from_slice(&shell.gfx.canvas.px[y * w..(y + 1) * w]);
                }
                let missing = shell.gfx.missing_icons();
                if !missing.is_empty() {
                    eprintln!("{name} {}/{}: icons not built in: {missing:?}", theme.name(), size.name());
                }
            }
        }
        crate::platform::sim::save_png(&sheet, &dir.join(format!("clip-{name}.png")), 2);
    }
    // The motion: quick capture grows out of the island over the Terminal,
    // is typed into, saves, and shrinks back into the island, which says
    // where it went; then the history comes and goes. Every frame at 30 a second.
    for theme in [Theme::Catalyst, Theme::Bezel] {
        fill(true);
        let mut shell = Shell::new(crate::robot::Robot::demo());
        shell.settings.theme = theme;
        (shell.settings.wallpaper, shell.settings.accent) = theme.defaults();
        shell.apply_settings();
        frames(&mut shell, 100);
        let (cols, rows) = crate::terminal::Terminal::grid_for(&mut shell.gfx);
        shell.show(Box::new(crate::terminal::Terminal::showing(printed, cols, rows, 0)));
        frames(&mut shell, 60);
        let mut film: Vec<Vec<u32>> = Vec::new();
        let roll = |shell: &mut Shell, n: usize, film: &mut Vec<Vec<u32>>| {
            for _ in 0..n {
                shell.tick(1.0 / 60.0);
                shell.draw();
                shell.tick(1.0 / 60.0);
                shell.draw();
                film.push(shell.gfx.canvas.px.clone());
            }
        };
        roll(&mut shell, 3, &mut film);
        shell.key(CAPTURE_KEY);
        roll(&mut shell, 14, &mut film);
        for c in "swap battery in 10 min".chars() {
            shell.key(Key::Char(c));
            roll(&mut shell, 1, &mut film);
        }
        roll(&mut shell, 8, &mut film);
        shell.key(Key::Enter);
        roll(&mut shell, 30, &mut film);
        shell.key(HISTORY_KEY);
        roll(&mut shell, 16, &mut film);
        shell.key(Key::Down);
        roll(&mut shell, 6, &mut film);
        shell.key(Key::Down);
        roll(&mut shell, 10, &mut film);
        shell.key(Key::Back);
        roll(&mut shell, 16, &mut film);
        crate::platform::sim::save_apng(&film, w, h, &dir.join(format!("clip-motion-{}.png", theme.name())), 2, 33);
        // The first frames of the sheet growing, as stills side by side.
        let grow: Vec<&Vec<u32>> = film.iter().skip(3).take(8).collect();
        let mut strip = bezel::Canvas::new((4 * w + 3 * gap) as i32, (2 * h + gap) as i32);
        strip.px.fill(0x202124);
        for (k, frame) in grow.iter().enumerate() {
            let (cx, cy) = (k % 4, k / 4);
            for y in 0..h {
                let at = (cy * (h + gap) + y) * (4 * w + 3 * gap) + cx * (w + gap);
                strip.px[at..at + w].copy_from_slice(&frame[y * w..(y + 1) * w]);
            }
        }
        crate::platform::sim::save_png(&strip, &dir.join(format!("clip-grow-{}.png", theme.name())), 2);
    }
    std::fs::remove_dir_all(&own).ok();
}

// Opened as the galleries open screens (`Shell::show`), which the ARM build leaves out.
#[cfg(all(test, not(target_arch = "aarch64")))]
mod tests {
    use super::*;
    use crate::shell::tests::sandbox;

    const FRAME: f32 = 1.0 / 60.0;

    /// A shell on the demo robot, drawn until Home has arrived.
    fn shell_for_tests() -> Shell {
        sandbox();
        let mut shell = Shell::new(Robot::demo());
        shell.apply_settings();
        for _ in 0..120 {
            shell.tick(FRAME);
            shell.draw();
        }
        shell
    }

    fn settle(shell: &mut Shell) {
        for _ in 0..90 {
            shell.tick(FRAME);
            shell.draw();
        }
    }

    #[test]
    fn ctrl_shift_n_captures_a_note_from_inside_the_terminal() {
        let mut shell = shell_for_tests();
        shell.show(Box::new(crate::terminal::Terminal::new()));
        settle(&mut shell);
        shell.key(CAPTURE_KEY);
        settle(&mut shell);
        assert_eq!(shell.sheet_open(), Some(Sheet::Capture));
        for c in "belt slips".chars() {
            shell.key(Key::Char(c));
        }
        assert_eq!(shell.sheets.capture.text(), "belt slips", "typed into the sheet, not the terminal");
        shell.key(Key::Enter);
        settle(&mut shell);
        assert_eq!(shell.sheet_open(), None);
        let said = shell.message.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(said.contains("notes"), "the island says where it went: {said}");
        let Some((Receipt { place: crate::capture::Place::Note(path), .. }, _)) = shell.sheets.receipt.clone() else { panic!("a note") };
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("belt slips\n"));
        // Ctrl-Enter meanwhile opens it there.
        shell.key(Key::Ctrl('\n'));
        settle(&mut shell);
        assert_eq!(shell.top_title(), path.file_name().unwrap().to_string_lossy());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn copy_and_paste_go_between_fields_and_rows() {
        let mut shell = shell_for_tests();
        crate::clipboard::copy("12*", Sensitivity::Normal);
        // Ctrl-V in a text field types it in; Ctrl-Shift-V too.
        shell.show(Box::new(crate::calc::Calculator::new()));
        settle(&mut shell);
        shell.key(Key::Ctrl('v'));
        shell.key(Key::Char('7'));
        shell.key(Key::Ctrl('C'));
        settle(&mut shell);
        assert_eq!(crate::clipboard::paste_text().as_deref(), Some("12*7"), "pasted, typed on, copied");
        shell.key(Key::Enter);
        shell.key(Key::Ctrl('c'));
        settle(&mut shell);
        assert_eq!(crate::clipboard::paste_text().as_deref(), Some("84"), "with nothing typed, the answer");
        // A table's value, whole.
        shell.key(Key::Back);
        shell.key(Key::Back);
        settle(&mut shell);
        shell.show(Box::new(crate::screens::Tables::at("Robot/Identity/Name", &shell.robot)));
        settle(&mut shell);
        shell.key(Key::Ctrl('c'));
        settle(&mut shell);
        let name = shell.robot.string("Robot/Identity/Name").expect("the demo robot has a name");
        assert_eq!(crate::clipboard::paste_text(), Some(name));
        // Ctrl-V where nothing takes text: nothing typed, and the island says why.
        shell.key(Key::Ctrl('v'));
        assert_eq!(shell.message.as_ref().map(|(m, _)| m.as_str()), Some("nothing here takes text"));
    }

    #[test]
    fn the_history_comes_up_from_anywhere_and_enter_pastes_into_the_field_in_front() {
        let mut shell = shell_for_tests();
        crate::clipboard::copy("older", Sensitivity::Normal);
        crate::clipboard::copy("newer", Sensitivity::Normal);
        shell.show(Box::new(crate::notes::Notes::new()));
        settle(&mut shell);
        shell.key(Key::Char('/'));
        settle(&mut shell);
        shell.key(HISTORY_KEY);
        settle(&mut shell);
        assert_eq!(shell.sheet_open(), Some(Sheet::Clipboard));
        shell.key(Key::Down);
        shell.key(Key::Enter);
        settle(&mut shell);
        assert_eq!(shell.sheet_open(), None);
        assert_eq!(shell.gfx.copy_field.as_deref(), Some("older"), "pasted into the search field");
        assert_eq!(crate::clipboard::paste_text().as_deref(), Some("older"), "and it is the clipboard now");
        // Over the control centre it comes up too, and the sheet gives way to Ctrl-Space.
        shell.key(Key::Ctrl(' '));
        settle(&mut shell);
        shell.key(HISTORY_KEY);
        settle(&mut shell);
        assert!(!shell.control_open() && shell.sheet_open() == Some(Sheet::Clipboard));
        shell.key(Key::Ctrl(' '));
        settle(&mut shell);
        assert!(shell.control_open() && shell.sheet_open().is_none());
    }

    #[test]
    fn a_secret_goes_when_the_handheld_locks() {
        let mut shell = shell_for_tests();
        crate::clipboard::copy("hunter2", Sensitivity::Secret);
        settle(&mut shell);
        let said = shell.message.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert_eq!(said, "copied · clears in 30 s");
        shell.lock_as_if_held();
        settle(&mut shell);
        assert_eq!(crate::clipboard::paste_text(), None, "gone at the lock");
    }
}
