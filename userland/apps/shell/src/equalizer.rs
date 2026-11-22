//! Settings › Sound › Equalizer: a preset (flat, bass, vocal, treble,
//! loudness, custom) and the five bands it sets, each moved by hand a
//! decibel at a time (which makes the preset custom, from where the bands
//! were). The player takes it within half a second (audiofx::settings), and
//! flat costs it nothing: the samples go by untouched.
//!
//! Loudness lifts the bass and treble as the volume comes down, as the ear
//! hears less of them quietly; its bands show what it does at this volume.

use audiofx::eq::{Preset, FREQS, LIMIT};
use bezel::Rect;

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::sound::{fx, set_fx};

/// The preset's row, then the five bands'.
const ROWS: usize = 1 + FREQS.len();

/// "60 Hz", "1 kHz", "12 kHz".
fn band_name(freq: f32) -> String {
    if freq >= 1000.0 {
        format!("{} kHz", (freq / 1000.0).round() as u32)
    } else {
        format!("{} Hz", freq.round() as u32)
    }
}

/// "+6 dB", "0 dB", "-3 dB".
fn db_words(db: f32) -> String {
    let d = db.round() as i32;
    if d > 0 {
        format!("+{d} dB")
    } else {
        format!("{d} dB")
    }
}

/// The bands as the preset sets them now (at this volume, for loudness).
fn gains() -> [f32; 5] {
    let f = fx();
    f.eq.gains(f.bands, crate::sound::level().volume)
}

pub struct EqualizerPage {
    list: ListState,
    /// Each band's bar, gliding to its gain.
    bars: Vec<detent::Animated>,
    changed: bool,
}

impl EqualizerPage {
    pub fn new() -> Self {
        let bars = gains().iter().map(|&g| detent::Animated::new(g / LIMIT as f32, detent::role::hold()).with_rest(0.003)).collect();
        EqualizerPage { list: ListState::default(), bars, changed: true }
    }

    fn describe(&self, i: usize) -> (String, String, bool) {
        let f = fx();
        if i == 0 {
            return ("Preset".into(), f.eq.word().into(), f.eq != Preset::Flat);
        }
        let g = gains()[i - 1];
        (band_name(FREQS[i - 1]), db_words(g), g.abs() >= 0.5)
    }

    fn hint(&self, i: usize) -> Vec<String> {
        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        match (i, fx().eq) {
            (0, Preset::Loudness) => words(&["←→: preset · more bass and treble when quiet", "←→: preset · louder when quiet", "←→: preset"]),
            (0, Preset::Flat) => words(&["←→: preset · flat: the song as it was made", "←→: preset · flat, untouched", "←→: preset"]),
            (0, _) => words(&["←→: flat, bass, vocal, treble, loudness, custom", "←→: preset"]),
            (i, _) => vec![format!("←→: {} a decibel at a time, to ±{LIMIT}", band_name(FREQS[i - 1])), format!("←→: ±1 dB, to ±{LIMIT}"), "←→: ±1 dB".into()],
        }
    }

    fn step_preset(&mut self, step: i32) -> String {
        let f = set_fx(|f| {
            let at = Preset::ALL.iter().position(|p| *p == f.eq).unwrap_or(0) as i32;
            f.eq = Preset::ALL[(at + step).rem_euclid(Preset::ALL.len() as i32) as usize];
        });
        format!("equalizer: {}", f.eq.word())
    }

    /// A band moved by hand: the preset becomes custom, from the bands as
    /// they were.
    fn step_band(&mut self, band: usize, step: i32) -> String {
        let now = gains();
        let f = set_fx(|f| {
            let mut bands = now.map(|g| g.round().clamp(-(LIMIT as f32), LIMIT as f32) as i8);
            bands[band] = (bands[band] as i32 + step).clamp(-(LIMIT as i32), LIMIT as i32) as i8;
            f.bands = bands;
            f.eq = Preset::Custom;
        });
        format!("{} {}", band_name(FREQS[band]), db_words(f.bands[band] as f32))
    }
}

impl Screen for EqualizerPage {
    fn title(&self) -> &str {
        "Equalizer"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let i = self.list.focus.min(ROWS - 1);
        let step = if key == Key::Left { -1 } else { 1 };
        match key {
            Key::Back => return Nav::Pop,
            Key::Left | Key::Right | Key::Enter if i == 0 => *cx.say = Some(self.step_preset(step)),
            Key::Left | Key::Right if i > 0 => *cx.say = Some(self.step_band(i - 1, step)),
            Key::Left | Key::Right => {}
            other => crate::screens::list_key(&mut self.list, other, ROWS),
        }
        self.changed = true;
        Nav::Stay
    }

    fn update(&mut self, _cx: &mut Cx) -> Nav {
        Nav::Stay
    }

    fn resume(&mut self) {
        self.changed = true;
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for (bar, g) in self.bars.iter_mut().zip(gains()) {
            let target = g / LIMIT as f32;
            if bar.target() != target {
                bar.set(target);
            }
            moving |= bar.tick(dt);
        }
        std::mem::take(&mut self.changed) | moving
    }

    fn busy(&self) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 5.0;
        // Every band's bar in the same place, so their middles line up.
        let name_w = FREQS.iter().map(|&f| band_name(f)).map(|n| g.measure(&n.to_lowercase(), g.ty.label).max(g.measure(&n, g.ty.body))).fold(0.0, f32::max) + 22.0;
        let value_w = g.measure("+12 dB", g.ty.body).max(g.measure("-12 dB", g.ty.label)) + 22.0;
        for (i, y) in self.list.layout(g, ROWS, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, width, row_h);
            let (name, value, lit) = self.describe(i);
            g.row(r, &name, &value, if lit { t.ink } else { t.dim }, i == self.list.focus);
            // A band's bar, either side of nothing, between its name and value.
            if i > 0 {
                let room = r.w - name_w - value_w;
                if room > 24.0 {
                    g.balance(Rect::new(r.x + name_w, r.cy() - 3.0, room, 6.0), self.bars[i - 1].value(), t.ice);
                }
            }
        }
        self.list.scrollbar(g, ROWS, top, bottom);
        let hint = self.hint(self.list.focus.min(ROWS - 1));
        let choices: Vec<&str> = hint.iter().map(String::as_str).collect();
        g.hint_any(&choices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_are_named_and_valued_plainly() {
        assert_eq!(FREQS.map(band_name), ["60 Hz", "250 Hz", "1 kHz", "4 kHz", "12 kHz"]);
        assert_eq!((db_words(6.0), db_words(0.2), db_words(-3.0)), ("+6 dB".into(), "0 dB".into(), "-3 dB".into()));
    }

    #[test]
    fn every_row_and_hint_fits_whole_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let page = EqualizerPage::new();
        let width = W - 2.0 * PAD - 5.0;
        for look in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            g.theme = look;
            for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
                g.set_text_size(size);
                let (name_style, value_style) = if look == crate::look::Theme::Bezel { (g.ty.label, g.ty.body) } else { (g.ty.body, g.ty.label) };
                for i in 0..ROWS {
                    let (name, _, _) = page.describe(i);
                    let name = if look == crate::look::Theme::Bezel { name.to_lowercase() } else { name };
                    // The widest a value gets, and the bar's least room.
                    let value = if i == 0 { "loudness".to_owned() } else { "-12 dB".to_owned() };
                    let need = g.measure(&name, name_style) + g.measure(&value, value_style) + 30.0 + if i > 0 { 24.0 + 44.0 } else { 0.0 };
                    assert!(need <= width, "{name} · {value} at {size:?} in {look:?}");
                    assert!(g.measure(page.hint(i).last().unwrap(), g.ty.caption) <= W - 2.0 * PAD);
                }
            }
        }
    }
}
