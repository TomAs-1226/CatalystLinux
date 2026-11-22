//! Settings › Music: what Music keeps on the card and how it downloads.
//!
//! - **Played songs**: how much the played cache holds, and its limit
//!   (←→: 1 to 64 GB). Past it, the songs played longest ago make way.
//! - **Saved songs**: the library's size; never let go by Music.
//! - **Clear played songs**: the cache emptied (Enter twice); saved songs stay.
//! - **Downloads**: only over Wi-Fi, only while charging, your likes saved
//!   by themselves (signed in).
//! - **Look for new music**: the library's index brought up to date with
//!   every file, for music copied in over SSH or with Files.

use bezel::Rect;

use super::{library, player, store};
use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Played,
    Saved,
    Clear,
    WifiOnly,
    Charging,
    Auto,
    Rescan,
}

const ROWS: [Row; 7] = [Row::Played, Row::Saved, Row::Clear, Row::WifiOnly, Row::Charging, Row::Auto, Row::Rescan];

pub struct MusicSettings {
    list: ListState,
    armed: bool,
    changed: bool,
    seen: u64,
}

/// Settings' one line for it: what the card holds.
pub fn summary() -> String {
    let (saved, _) = library::bytes();
    format!("{} kept", store::size_words(saved + store::cache_bytes()))
}

impl MusicSettings {
    pub fn new() -> Self {
        MusicSettings { list: ListState::default(), armed: false, changed: true, seen: 0 }
    }

    /// Opened at one of its rows (from search).
    pub fn at(mut self, row: usize) -> Self {
        self.list.focus = row.min(ROWS.len() - 1);
        self
    }

    fn describe(&self, row: Row) -> (&'static str, String, bool) {
        let c = store::choices();
        let onoff = |b: bool| (if b { "on" } else { "off" }).to_owned();
        match row {
            Row::Played => ("Played songs", format!("{} of {}", store::size_words(store::cache_bytes()), store::size_words(c.cache_limit())), true),
            Row::Saved => {
                let (bytes, n) = library::bytes();
                ("Saved songs", format!("{} · {n}", store::size_words(bytes)), true)
            }
            Row::Clear => ("Clear played songs", (if self.armed { "enter again" } else { "" }).into(), true),
            Row::WifiOnly => ("Download only over Wi-Fi", onoff(c.wifi_only), c.wifi_only),
            Row::Charging => ("Download only while charging", onoff(c.charging_only), c.charging_only),
            Row::Auto => ("Save your likes by themselves", onoff(c.auto), c.auto),
            Row::Rescan => ("Look for new music", (if library::refreshing() { "looking…" } else { "" }).into(), true),
        }
    }

    fn hint(&self, row: Row) -> &'static [&'static str] {
        match row {
            Row::Played => &["←→: the limit · the oldest make way past it", "←→: the limit"],
            Row::Saved => &["in /data/music/library · never let go", "never let go"],
            Row::Clear if self.armed => &["enter again: clear · saved songs stay", "enter again: clear"],
            Row::Clear => &["played songs go · saved songs stay", "saved songs stay"],
            Row::WifiOnly => &["not over a computer's USB link", "enter: on or off"],
            Row::Charging => &["downloads wait for the charger", "enter: on or off"],
            Row::Auto => &["signed in, over wi-fi, twice a day", "enter: on or off"],
            Row::Rescan => &["for music copied in over ssh or files", "enter: look"],
        }
    }
}

impl Screen for MusicSettings {
    fn title(&self) -> &str {
        "Music"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        self.changed = true;
        let row = ROWS[self.list.focus.min(ROWS.len() - 1)];
        let armed = std::mem::take(&mut self.armed);
        let mut c = store::choices();
        match key {
            Key::Back => return Nav::Pop,
            Key::Left | Key::Right if row == Row::Played => {
                let at = store::CACHE_LIMITS.iter().position(|&g| g == c.cache_gb).unwrap_or(2) as i32 + if key == Key::Left { -1 } else { 1 };
                c.cache_gb = store::CACHE_LIMITS[at.clamp(0, store::CACHE_LIMITS.len() as i32 - 1) as usize];
                store::set_choices(c);
                let n = player::now();
                let keep: Vec<String> = n.queue.iter().skip(n.at).take(8).map(|t| t.id.clone()).collect();
                store::evict(&keep);
                *cx.say = Some(format!("played songs: up to {}", store::size_words(c.cache_limit())));
            }
            Key::Enter if row == Row::Clear => {
                if armed {
                    let n = player::now();
                    let keep: Vec<String> = n.track().map(|t| vec![t.id.clone()]).unwrap_or_default();
                    let freed = store::clear_cache(&keep);
                    *cx.say = Some(format!("{} cleared", store::size_words(freed)));
                } else {
                    self.armed = true;
                }
            }
            Key::Enter | Key::Left | Key::Right if row == Row::WifiOnly => {
                c.wifi_only = !c.wifi_only;
                store::set_choices(c);
            }
            Key::Enter | Key::Left | Key::Right if row == Row::Charging => {
                c.charging_only = !c.charging_only;
                store::set_choices(c);
            }
            Key::Enter | Key::Left | Key::Right if row == Row::Auto => {
                c.auto = !c.auto;
                store::set_choices(c);
            }
            Key::Enter if row == Row::Rescan => {
                library::refresh(true);
                *cx.say = Some("looking for new music".into());
            }
            other => crate::screens::list_key(&mut self.list, other, ROWS.len()),
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        let now = library::stamp() + store::played_stamp();
        if now != self.seen {
            self.seen = now;
            self.changed = true;
        }
        std::mem::take(&mut self.changed)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 5.0;
        for (i, y) in self.list.layout(g, ROWS.len(), top, row_h, 3.0, bottom) {
            let (name, value, lit) = self.describe(ROWS[i]);
            g.row(Rect::new(PAD, y, width, row_h), name, &value, if lit { t.ink } else { t.dim }, i == self.list.focus);
        }
        self.list.scrollbar(g, ROWS.len(), top, bottom);
        g.hint_any(self.hint(ROWS[self.list.focus.min(ROWS.len() - 1)]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_and_hint_fits_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let s = MusicSettings::new();
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            for row in ROWS {
                let (name, value, _) = s.describe(row);
                assert!(g.measure(name, ty.body) + g.measure(&value, ty.body) + 40.0 <= W - 2.0 * PAD, "{name} {value} at {size:?}");
                assert!(s.hint(row).iter().any(|h| g.measure(h, ty.caption) <= W - 2.0 * PAD), "{row:?} at {size:?}");
            }
        }
    }
}
