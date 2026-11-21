//! Every key a running universe answers, for when the hint line has room
//! for only some of them: two tiles of keycaps and what each does.

use catalyst_kit::prelude::*;
use catalyst_kit::Theme;

/// The keys, as printed on the handheld, what they do, and the same in a
/// word for when the words will not fit (the large text size).
const KEYS: [[(&str, &str, &str); 4]; 2] = [
    [("space", "pause, play", "pause"), ("←→", "slower, faster", "time"), ("↑↓", "zoom in, out", "zoom"), ("tab", "follow a body", "follow")],
    [("n", "set one loose", "new body"), ("r", "start again", "restart"), ("h", "these keys", "keys"), ("esc", "the universes", "back")],
];

/// Room inside a tile, either side, and between a keycap and its words.
const INSET: f32 = 8.0;
const SPACE: f32 = 7.0;

/// How the page is laid out at the text size in use: every key's words in
/// full if both tiles have room, otherwise every key's word. Never cut.
pub struct Plan {
    pub long: bool,
    /// Each tile's width, and its keycaps' width.
    pub tiles: [f32; 2],
    pub caps: [f32; 2],
}

impl Plan {
    pub fn of(ui: &mut Ui) -> Plan {
        let room = W - 2.0 * PAD - GAP;
        let (long, (total, caps)) = match Plan::need(ui, true) {
            (t, caps) if t[0] + t[1] <= room => (true, (t, caps)),
            _ => (false, Plan::need(ui, false)),
        };
        // The room left over is shared out evenly.
        let extra = ((room - total[0] - total[1]) * 0.5).max(0.0);
        Plan { long, tiles: [total[0] + extra, total[1] + extra], caps }
    }

    /// Each tile's least width, and its keycaps' width.
    fn need(ui: &mut Ui, long: bool) -> ([f32; 2], [f32; 2]) {
        let ty = ui.ty;
        let (mut total, mut caps) = ([0.0; 2], [0.0f32; 2]);
        for (c, column) in KEYS.iter().enumerate() {
            let mut widest = 0.0f32;
            for (key, l, s) in column {
                caps[c] = caps[c].max((ui.measure(key, ty.caption) + 8.0).max(ty.caption.size + 5.0));
                let said = words(ui, if long { l } else { s });
                widest = widest.max(ui.measure(&said, ty.body));
            }
            total[c] = 2.0 * INSET + caps[c] + SPACE + widest + 1.0;
        }
        (total, caps)
    }

    /// Whether every key's words fit their tile.
    #[cfg(test)]
    pub fn fits(&self) -> bool {
        self.tiles[0] + self.tiles[1] <= W - 2.0 * PAD - GAP + 0.5
    }
}

fn words(ui: &Ui, s: &str) -> String {
    if ui.look.theme == Theme::Bezel {
        s.to_lowercase()
    } else {
        s.to_owned()
    }
}

pub struct Help;

impl Page for Help {
    fn title(&self) -> &str {
        "Keys"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Flow {
        match key {
            Key::Back | Key::Enter | Key::Char('h') | Key::Char('H') | Key::Char('?') | Key::F(1) => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (t, ty) = (ui.tokens, ui.ty);
        let (top, bottom) = (ui.top(), ui.bottom());
        let plan = Plan::of(ui);
        let radius = ui.tile_radius();
        let pitch = (bottom - top - 8.0) / 4.0;
        let mut x = PAD;
        for (c, column) in KEYS.iter().enumerate() {
            let tile = Rect::new(x, top, plan.tiles[c], bottom - top);
            x += plan.tiles[c] + GAP;
            ui.surface(tile, radius, t.surface1);
            for (r, (cap, long, short)) in column.iter().enumerate() {
                let cy = tile.y + 4.0 + pitch * (r as f32 + 0.5);
                ui.keycap(tile.x + INSET, cy, cap);
                let at = tile.x + INSET + plan.caps[c] + SPACE;
                let said = words(ui, if plan.long { long } else { short });
                ui.text_at(at, cy, Align::Left, &said, ty.body, t.ink);
            }
        }
        ui.hint("esc: back");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalyst_kit::{Look, TextSize};

    #[test]
    fn every_key_is_written_whole_at_every_text_size() {
        for text in TextSize::ALL {
            for theme in [catalyst_kit::Theme::Catalyst, catalyst_kit::Theme::Bezel] {
                let mut ui = Ui::new(Look { text, theme, ..Look::default() });
                let plan = Plan::of(&mut ui);
                assert!(plan.fits(), "{text:?} {theme:?}: tiles {:?}", plan.tiles);
            }
        }
        let mut ui = Ui::new(Look::default());
        assert!(Plan::of(&mut ui).long, "at the standard size, the words are written out");
    }
}
