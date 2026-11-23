//! The sheet of a screen's keys: `?` tapped over any screen not taking
//! text (Fn 1 where it does) grows it out of the island, as the clipboard's
//! history is (`sheet.rs`), listing what the screen's own keys do
//! (`Screen::key_sheet`) and then the keys that work everywhere.
//!
//! ↑ ↓ scroll it, a page at a time with Pg Up and Pg Dn; Esc, Enter or `?`
//! put it away. Any other key puts it away at once and goes on to the
//! screen, so a key looked up can be pressed straight from the sheet.
//! `?` held on for five seconds goes on to every shortcut (`help.rs`).

use bezel::Rect;
use detent::Animated;

use crate::gfx::Gfx;
use crate::help::KeyGroup;
use crate::platform::Key;
use crate::sheet::{keys_line, keys_shown, text_right, Grown, INSET};

/// A line of the sheet: a group's heading, or a key and what it does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Line {
    Head(&'static str),
    Row(&'static str, &'static str),
}

/// What a key did to the sheet.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Taken: it scrolled, or did nothing.
    Stay,
    /// Put away.
    Close,
    /// Put away at once, and the key goes on to the screen.
    Pass(Key),
}

/// The sheet's measures at the text size: its bar, a line, the keys along
/// its foot.
fn metrics(g: &Gfx) -> (f32, f32, f32) {
    let bar = (g.ty.name.size + 15.0).round();
    let line = (g.ty.caption.size + 10.0).round();
    let keys = (g.ty.caption.size + 10.0).round();
    (bar, line, keys)
}

pub struct KeySheet {
    sheet: Grown,
    /// Whose keys: the screen's title.
    title: String,
    lines: Vec<Line>,
    /// The first line in sight, and where the list has glided to.
    first: usize,
    scroll: Animated,
    placed: bool,
    /// How many lines the sheet shows at once, as last drawn.
    in_sight: usize,
    /// What holding `?` on will do, said in the bar.
    pub note: Option<String>,
}

impl Default for KeySheet {
    fn default() -> Self {
        Self::new()
    }
}

impl KeySheet {
    pub fn new() -> Self {
        KeySheet {
            sheet: Grown::new(),
            title: String::new(),
            lines: Vec::new(),
            first: 0,
            scroll: Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.003),
            placed: false,
            in_sight: 4,
            note: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.sheet.is_open()
    }

    pub fn visible(&self) -> bool {
        self.sheet.visible()
    }

    pub fn shown(&self) -> f32 {
        self.sheet.shown()
    }

    pub fn dim(&self) -> f32 {
        self.sheet.dim()
    }

    pub fn grown(&self) -> &Grown {
        &self.sheet
    }

    pub fn glass(&self, g: &Gfx, island: Rect) -> bezel::GlassShape {
        self.sheet.glass(g, island)
    }

    /// Whose keys it shows (the tests).
    #[allow(dead_code)]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Every row it lists, (keys, what they do), for the tests.
    #[allow(dead_code)]
    pub fn rows(&self) -> Vec<(&'static str, &'static str)> {
        self.lines.iter().filter_map(|l| if let Line::Row(k, w) = l { Some((*k, *w)) } else { None }).collect()
    }

    /// Bring it up with `title`'s keys, `own` then those that work
    /// everywhere, in place of another sheet `over` if that is showing.
    pub fn summon(&mut self, g: &Gfx, calm: bool, title: &str, own: &[KeyGroup], over: Option<&Grown>) {
        self.title = title.to_owned();
        self.lines = own.iter().chain(std::iter::once(&crate::help::EVERYWHERE)).flat_map(|group| std::iter::once(Line::Head(group.title)).chain(group.keys.iter().map(|&(k, w)| Line::Row(k, w)))).collect();
        self.first = 0;
        self.placed = false;
        self.note = None;
        let height = self.height_for(g);
        self.sheet.summon(calm, height, over);
    }

    pub fn dismiss(&mut self) {
        self.sheet.dismiss();
        self.note = None;
    }

    pub fn gone_now(&mut self) {
        self.sheet.gone_now();
        self.note = None;
    }

    fn height_for(&self, g: &Gfx) -> f32 {
        let (bar, line, keys) = metrics(g);
        bar + 4.0 + self.lines.len() as f32 * line + 4.0 + keys
    }

    /// The last line that can be first in sight.
    fn last_first(&self) -> usize {
        self.lines.len().saturating_sub(self.in_sight.max(1))
    }

    pub fn key(&mut self, key: Key) -> Outcome {
        // Nothing is typed here: the letters that are arrows outside typing are arrows.
        let key = match key {
            Key::Char('f') => Key::Up,
            Key::Char('x') => Key::Down,
            other => other,
        };
        let page = self.in_sight.max(2) - 1;
        match key {
            Key::Up => self.first = self.first.saturating_sub(1),
            Key::Down => self.first = (self.first + 1).min(self.last_first()),
            Key::PageUp => self.first = self.first.saturating_sub(page),
            Key::PageDown => self.first = (self.first + page).min(self.last_first()),
            Key::Home => self.first = 0,
            Key::End => self.first = self.last_first(),
            Key::Back | Key::Backspace | Key::Enter | Key::Leave => return Outcome::Close,
            // Turning past either end of the list: nothing.
            Key::Left | Key::Right | Key::Char('z' | 'c') => {}
            other => return Outcome::Pass(other),
        }
        Outcome::Stay
    }

    pub fn tick(&mut self, dt: f32) -> bool {
        self.sheet.tick(dt) | self.scroll.tick(dt)
    }

    pub fn draw(&mut self, g: &mut Gfx, island: Rect) {
        let (bar_h, line_h, keys_h) = metrics(g);
        self.sheet.set_height(g, self.height_for(g));
        let panel = self.sheet.glass(g, island).rect;
        let words = self.sheet.words();
        if words <= 0.0 {
            return;
        }
        let t = g.tokens;
        let ty = g.ty;
        g.canvas.set_clip(panel.inset(1.0));
        // The bar: whose keys; beside it, what holding ? on will do.
        let bar_cy = panel.y + bar_h * 0.5;
        g.icon_alpha("keyboard", false, panel.x + 16.0, bar_cy, 15.0, t.dim, words);
        let title_room = panel.w - 30.0 - 14.0;
        let title = g.text.fit(&self.title, &ty.name, title_room * 0.6);
        g.text_alpha(panel.x + 30.0, bar_cy, &title, ty.name, t.ink, words);
        let title_end = panel.x + 30.0 + g.measure(&title, ty.name) + 10.0;
        let room = (panel.right() - 14.0 - title_end).max(0.0);
        let said = match &self.note {
            Some(note) => g.text.fit(note, &ty.caption, room),
            None => crate::sheet::first_fitting(g, &["this screen's keys", "its keys", ""], ty.caption, room).to_owned(),
        };
        text_right(g, panel.right() - 14.0, bar_cy, &said, ty.caption, if self.note.is_some() { t.ink } else { t.dim }, words);
        // The list, as it rests, clipped to the glass as it grows.
        let rest_h = self.sheet.rest().h;
        let list = Rect::new(panel.x, panel.y + bar_h, panel.w, (rest_h - keys_h - bar_h).max(0.0));
        if list.h > 1.0 {
            g.canvas.rrect(Rect::new(panel.x + 12.0, list.y, panel.w - 24.0, 1.0), 0.5, t.ink, 0.10 * words);
            self.in_sight = ((list.h - 8.0) / line_h).floor().max(1.0) as usize;
            self.first = self.first.min(self.last_first());
            let target = self.first as f32;
            if !self.placed || self.sheet.calm {
                self.scroll.jump(target);
                self.placed = true;
            } else if self.scroll.target() != target {
                self.scroll.set(target);
                g.started_moving();
            }
            // Only whole lines at rest: never one cut by the sheet's foot.
            let top = list.y + 4.0;
            g.canvas.set_clip(Rect::new(list.x + 1.0, list.y + 1.0, list.w - 2.0, (top + self.in_sight as f32 * line_h - list.y - 1.0).min(list.h - 2.0)));
            let scroll = self.scroll.value();
            let (x, w) = (list.x + INSET, list.w - 2.0 * INSET - 6.0);
            let from = scroll.floor().max(0.0) as usize;
            for i in from..(from + self.in_sight + 2).min(self.lines.len()) {
                let y = top + (i as f32 - scroll) * line_h;
                let cy = y + line_h * 0.5;
                match self.lines[i] {
                    Line::Head(name) => {
                        g.text_alpha(x + 6.0, cy + 1.0, name, ty.caption, t.dim, words);
                    }
                    Line::Row(keys, what) => {
                        let caps_left = crate::help::keycaps(g, x + w - 6.0, cy, keys, false, false);
                        let style = if g.theme == crate::look::Theme::Bezel { ty.label } else { ty.body };
                        let what = if g.theme == crate::look::Theme::Bezel { what.to_lowercase() } else { what.to_owned() };
                        let what = g.text.fit(&what, &style, (caps_left - 6.0 - (x + 6.0)).max(0.0));
                        g.text_alpha(x + 6.0, cy, &what, style, t.ink, words);
                        crate::help::keycaps(g, x + w - 6.0, cy, keys, false, true);
                    }
                }
            }
            // Where it is along the list, when it runs past the sheet.
            if self.lines.len() > self.in_sight {
                let track = Rect::new(list.right() - INSET - 3.0, list.y + 5.0, 2.0, list.h - 10.0);
                let shown = self.in_sight as f32 / self.lines.len() as f32;
                let thumb_h = (track.h * shown).max(10.0);
                let along = scroll / self.last_first().max(1) as f32;
                let thumb = Rect::new(track.x, track.y + (track.h - thumb_h) * along.clamp(0.0, 1.0), 2.0, thumb_h);
                g.canvas.rrect(track, 1.0, t.ink, 0.08 * words);
                g.canvas.rrect(thumb, 1.0, t.ink, 0.45 * words);
            }
            g.canvas.set_clip(panel.inset(1.0));
        }
        // The keys along its foot, in once the glass reaches them.
        let keys_cy = panel.y + rest_h - keys_h * 0.5 - 1.0;
        let more = self.lines.len() > self.in_sight;
        let keys: &[(&str, &str)] = if more { &[("↑↓", "more"), ("esc", "close"), ("?", "held: every shortcut")] } else { &[("esc", "close"), ("?", "held: every shortcut")] };
        keys_line(g, panel.x + 12.0, panel.right() - 12.0, keys_cy, keys, words * keys_shown(panel, keys_cy, keys_h));
        g.canvas.clear_clip();
    }
}

/// `--keys-shots <dir>`: the sheets of keys over several apps, the hint
/// lines that say how they come up, and the Camera's bar with each of its
/// choices (camera/bar.rs), dark and light at the three text sizes, each
/// alone and on a sheet of all six (sizes across, dark over light).
#[cfg(not(target_arch = "aarch64"))]
pub fn shots(dir: &std::path::Path) {
    use crate::gfx::{TextSize, H, W};
    use crate::shell::Shell;
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    // (name, app, keys pressed once it is up)
    let apps: &[(&str, &str, &[Key])] = &[
        ("20-logs-hint", "Logs", &[]),
        ("21-logs-keys", "Logs", &[Key::Char('?')]),
        ("22-photos-keys", "Photos", &[Key::Char('?')]),
        // Its search row takes text as soon as it has the focus: Fn 1.
        ("23-music-keys-fn-1", "Music", &[Key::F(1)]),
        ("24-scout-keys", "Scout", &[Key::Char('?')]),
        ("25-strategy-keys", "Strategy", &[Key::F(1)]),
        ("25b-strategy-board-chips", "Strategy", &[]),
        ("26-batteries-keys", "Batteries", &[Key::Char('?')]),
        ("27-team-keys", "Team", &[Key::Char('?')]),
        ("28-rulebook-keys", "Rulebook", &[Key::Char('?')]),
        ("29-mech-keys", "Mech", &[Key::Char('?')]),
        ("30-gamepad-keys", "Gamepad", &[Key::Char('?')]),
        ("31-settings-keys", "Settings", &[Key::Char('?')]),
        ("32-settings-keys-everywhere", "Settings", &[Key::Char('?'), Key::End]),
        ("33-calculator-hint", "Calculator", &[]),
        ("34-calculator-keys-fn-1", "Calculator", &[Key::F(1)]),
    ];
    let mut scenes = crate::camera::bar_shots();
    for (name, app, keys) in apps {
        let mut shots = Vec::new();
        for light in [false, true] {
            for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
                let mut shell = Shell::new(crate::robot::Robot::demo());
                (shell.settings.text, shell.settings.light) = (size, light);
                shell.apply_settings();
                for _ in 0..30 {
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                }
                let (index, _, _) = crate::screens::builtin_apps().find(|(_, _, n)| n == app).expect("a built-in app");
                let screen = crate::screens::open_builtin(index, &shell.robot);
                shell.show(screen);
                for _ in 0..90 {
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
                shots.push(shell.gfx.canvas.clone());
            }
        }
        scenes.push(((*name).to_owned(), shots));
    }
    let (w, h) = (W as i32, H as i32);
    for (name, shots) in &scenes {
        for (i, shot) in shots.iter().enumerate() {
            let tone = if i < 3 { "dark" } else { "light" };
            let size = ["compact", "standard", "large"][i % 3];
            crate::platform::sim::save_png(shot, &dir.join(format!("{name}-{tone}-{size}.png")), 3);
        }
        let gap = 6;
        let mut sheet = bezel::Canvas::new(3 * w + 2 * gap, 2 * h + gap);
        sheet.px.fill(0x0080_8080);
        for (i, shot) in shots.iter().enumerate() {
            let (ox, oy) = ((i % 3) as i32 * (w + gap), (i / 3) as i32 * (h + gap));
            for y in 0..h {
                let from = (y * shot.w) as usize;
                let to = ((oy + y) * sheet.w + ox) as usize;
                sheet.px[to..to + w as usize].copy_from_slice(&shot.px[from..from + w as usize]);
            }
        }
        crate::platform::sim::save_png(&sheet, &dir.join(format!("sheet-{name}.png")), 2);
    }
    println!("key sheets and the camera's bar in {}", dir.display());
}

/// Every row of a sheet fits its line whole, words and keys, at every text
/// size, with room to spare for the sheet's edges (the tests).
#[cfg(test)]
pub fn fits(g: &mut Gfx, groups: &[KeyGroup]) -> Result<(), String> {
    use crate::gfx::{TextSize, W};
    for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
        g.set_text_size(size);
        let ty = g.ty;
        let w = W - 2.0 * crate::sheet::SIDE - 2.0 * INSET - 6.0;
        for group in groups {
            if group.keys.is_empty() {
                return Err(format!("{}: no keys", group.title));
            }
            for (k, what) in group.keys {
                let caps = w - 6.0 - crate::help::keycaps(g, w - 6.0, 0.0, k, false, false);
                let words = g.measure(what, ty.body).max(g.measure(&what.to_lowercase(), ty.label));
                if 6.0 + words + 6.0 + caps > w {
                    return Err(format!("{size:?}: \"{what}\" and \"{k}\" do not fit one line ({:.0} of {w:.0})", 12.0 + words + caps));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::help::keys;

    #[test]
    fn it_lists_the_screens_keys_then_those_that_work_everywhere_and_scrolls() {
        let g = Gfx::new(bezel::Tokens::dark());
        let mut sheet = KeySheet::new();
        const OWN: &[(&str, &str)] = &[("enter", "Open"), ("d", "Delete")];
        sheet.summon(&g, true, "Files", &[keys("files", OWN)], None);
        assert!(sheet.is_open());
        let rows = sheet.rows();
        assert_eq!(&rows[..2], OWN);
        assert!(rows.contains(&("ctrl tab", "Recent apps")), "then everywhere's");
        sheet.in_sight = 4;
        for _ in 0..40 {
            assert_eq!(sheet.key(Key::Down), Outcome::Stay);
        }
        assert_eq!(sheet.first, sheet.last_first(), "no further than the end");
        assert_eq!(sheet.key(Key::Char('f')), Outcome::Stay);
        assert_eq!(sheet.first, sheet.last_first() - 1, "f is up here");
        assert_eq!(sheet.key(Key::Char('d')), Outcome::Pass(Key::Char('d')), "a key looked up goes on to the screen");
        assert_eq!(sheet.key(Key::Back), Outcome::Close);
    }

    #[test]
    fn the_keys_that_work_everywhere_fit() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        fits(&mut g, &[crate::help::EVERYWHERE]).unwrap();
    }
}
