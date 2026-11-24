//! The Catalyst identity on the panel, after FrcCatalyst's own: the bolt of
//! `docs/assets/logo.svg`, the dark ground it sits on and its crimson glow,
//! and the wordmark of `docs/design.md`: a bold name beside a thin word, with
//! a hairline rule under it that is crimson at the left and gone by the
//! right. The brand keeps its own crimson whatever accent the UI wears;
//! nothing else here is crimson.

use bezel::canvas::blend;
use bezel::{Color, TextStyle};

use crate::gfx::{Gfx, H, W};

/// The mark's colour, `#e94560`.
pub const CRIMSON: Color = Color::hex(0xe94560);
/// The strongest text: the wordmark's name.
pub const INK_STRONG: Color = Color::hex(0xffffff);
/// The wordmark's thin word, and quiet captions.
pub const INK_THIN: Color = Color::hex(0x8d919a);
pub const INK_FAINT: Color = Color::hex(0x5d616a);

/// The mark's outline in the 24-unit box of the logo's artwork:
/// `M13.5 2 5 13h5l-1.5 9L19 10h-5.5z`.
const BOLT: [(f32, f32); 6] = [(13.5, 2.0), (5.0, 13.0), (10.0, 13.0), (8.5, 22.0), (19.0, 10.0), (13.5, 10.0)];

/// The mark, `height` pixels tall, centred on (`cx`, `cy`).
pub fn mark(g: &mut Gfx, cx: f32, cy: f32, height: f32, color: Color, alpha: f32) {
    // The artwork spans 2 to 22 down and 5 to 19 across, centred on 12.
    let s = height / 20.0;
    let pts: Vec<(f32, f32)> = BOLT.iter().map(|(x, y)| (cx + (x - 12.0) * s, cy + (y - 12.0) * s)).collect();
    g.canvas.polygon(&pts, color, alpha);
}

/// The logo's ground, its diagonal's stops: `#191a20` through `#101116`
/// to `#08090b` (the phone gallery's page draws its mark on them too).
pub const GROUND_STOPS: [(f32, Color); 3] = [(0.0, Color::hex(0x191a20)), (0.55, Color::hex(0x101116)), (1.0, Color::hex(0x08090b))];

/// The mark's outline as an SVG path, in the logo's 24-unit box.
pub const BOLT_PATH: &str = "M13.5 2 5 13h5l-1.5 9L19 10h-5.5z";

/// The logo's ground for the whole panel: the diagonal of [`GROUND_STOPS`],
/// with a faint pool of light at the top left.
pub fn ground() -> Vec<u32> {
    let stops = GROUND_STOPS;
    let mut px = Vec::with_capacity((W * H) as usize);
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = ((fx / W + fy / H) * 0.5).clamp(0.0, 1.0);
            let color = if t < stops[1].0 {
                stops[0].1.mix(stops[1].1, t / stops[1].0)
            } else {
                stops[1].1.mix(stops[2].1, (t - stops[1].0) / (1.0 - stops[1].0))
            };
            // The pool: an ellipse over the panel, centred a quarter in and a fifth down.
            let (dx, dy) = ((fx / W - 0.26) / 0.64, (fy / H - 0.20) / 0.64);
            let pool = (1.0 - (dx * dx + dy * dy).sqrt()).max(0.0) * 0.08;
            px.push(blend(color.to_u32(), Color::WHITE, pool));
        }
    }
    px
}

pub fn name_style() -> TextStyle {
    TextStyle { tracking: -0.02, ..TextStyle::sans(26.0, 700.0) }
}

pub fn thin_style() -> TextStyle {
    TextStyle { tracking: -0.01, ..TextStyle::sans(26.0, 300.0) }
}

/// How wide the wordmark is, and where its second word starts from its left.
pub fn wordmark_width(g: &mut Gfx) -> (f32, f32) {
    let name = g.measure("Catalyst", name_style());
    let gap = 6.0;
    (name + gap + g.measure("Linux", thin_style()), name + gap)
}

/// "Catalyst Linux", centred on `cx` and `cy`. Each word has its own opacity
/// and its own rise (pixels below its place), so they can arrive one after
/// the other.
pub fn wordmark(g: &mut Gfx, cx: f32, cy: f32, name: (f32, f32), thin: (f32, f32)) {
    let (width, second) = wordmark_width(g);
    let left = (cx - width * 0.5).round();
    if name.0 > 0.0 {
        g.text_alpha(left, cy + name.1, "Catalyst", name_style(), INK_STRONG, name.0.min(1.0));
    }
    if thin.0 > 0.0 {
        g.text_alpha(left + second, cy + thin.1, "Linux", thin_style(), INK_THIN, thin.0.min(1.0));
    }
}

/// The hairline under a wordmark: crimson at the left, gone by the right.
/// `drawn` (0 to 1) is how much of it has been drawn, from the left.
pub fn rule(g: &mut Gfx, x: f32, y: f32, width: f32, drawn: f32, alpha: f32) {
    let shown = (width * drawn.clamp(0.0, 1.0)).round() as i32;
    let (x0, row) = (x.round() as i32, y.round() as i32);
    if !(0..H as i32).contains(&row) {
        return;
    }
    for i in 0..shown {
        let px = x0 + i;
        if !(0..W as i32).contains(&px) {
            continue;
        }
        let fade = 1.0 - i as f32 / width.max(1.0);
        let at = (row * W as i32 + px) as usize;
        g.canvas.px[at] = blend(g.canvas.px[at], CRIMSON, alpha * fade * 0.9);
    }
}
