//! Every key a running universe answers, for when the hint line has room
//! for only some of them: two tiles of keycaps and what each does.

use catalyst_kit::prelude::*;
use catalyst_kit::Theme;

/// The keys, as printed on the handheld, and what they do.
const KEYS: [[(&str, &str); 4]; 2] = [
    [("space", "pause or play"), ("←→", "slower, faster"), ("↑↓", "zoom in, out"), ("tab", "follow a body")],
    [("n", "set one loose"), ("r", "start again"), ("h", "these keys"), ("esc", "the universes")],
];

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
        let tile_w = (W - 2.0 * PAD - GAP) * 0.5;
        let radius = ui.tile_radius();
        let bezel = ui.look.theme == Theme::Bezel;
        let inset = 9.0;
        let pitch = (bottom - top - 2.0 * 4.0) / 4.0;
        for (c, column) in KEYS.iter().enumerate() {
            let tile = Rect::new(PAD + c as f32 * (tile_w + GAP), top, tile_w, bottom - top);
            ui.surface(tile, radius, t.surface1);
            // The keycaps line up on the widest in the tile.
            let cap_w = column.iter().map(|(k, _)| (ui.measure(k, ty.caption) + 8.0).max(ty.caption.size + 5.0)).fold(0.0, f32::max);
            for (r, (cap, does)) in column.iter().enumerate() {
                let cy = tile.y + 4.0 + pitch * (r as f32 + 0.5);
                ui.keycap(tile.x + inset, cy, cap);
                let x = tile.x + inset + cap_w + 7.0;
                let words = if bezel { does.to_lowercase() } else { (*does).to_owned() };
                ui.text_fit(x, cy, &words, ty.body, t.ink, tile.right() - inset - x);
            }
        }
        ui.hint("esc: back");
    }
}
