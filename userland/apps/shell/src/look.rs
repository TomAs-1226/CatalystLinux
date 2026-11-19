//! How the handheld looks, beyond its colours: the theme, the wallpaper
//! behind every screen, the corners and the letters.
//!
//! Two themes. **Catalyst** is setup's look carried through: the brand's
//! deep ground with its pool of light, crimson, a white focus ring, rows
//! shaped as capsules and titles a step larger. **Bezel** is the house
//! style as the reference panels have it: the signal-coloured focus ring
//! and tonal tiles on a plain ground with two soft glows. Choosing a theme
//! also sets its own wallpaper and accent, which can then be changed.
//!
//! The wallpaper is drawn once for each palette, larger than the panel, so
//! that it can move behind the screens without being drawn again: a little
//! as Home's pages turn (parallax), and drifting after a key before it
//! settles (living wallpaper).

use bezel::canvas::blend;
use bezel::{Accent, Color, Tokens, Tone};

use crate::gfx::{H, W};

/// How far the wallpaper reaches past the panel on each side: what parallax
/// and drift have to move in.
pub const REACH: (usize, usize) = (48, 12);
/// The wallpaper's size.
pub const WALL: (usize, usize) = (W as usize + 2 * REACH.0, H as usize + 2 * REACH.1);

macro_rules! named {
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
            /// The next one along, or the one before (`step` −1).
            pub fn step(self, step: i32) -> Self {
                let at = Self::ALL.iter().position(|v| *v == self).unwrap_or(0) as i32;
                Self::ALL[(at + step).rem_euclid(Self::ALL.len() as i32) as usize]
            }
        }
    };
}

named!(Theme { Catalyst => "catalyst", Bezel => "bezel" });
named!(Wallpaper { Catalyst => "catalyst", Glow => "glow", Aurora => "aurora", Plain => "plain", Deep => "deep" });
named!(Corners { Round => "round", Standard => "standard", Square => "square" });
named!(Letters { Crisp => "crisp", Soft => "soft", Round => "round" });
named!(Width { Narrow => "narrow", Normal => "normal", Wide => "wide" });

/// Every choice of look together, as drawing needs them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Look {
    pub theme: Theme,
    pub wallpaper: Wallpaper,
    pub corners: Corners,
    pub letters: Letters,
    pub width: Width,
}

impl Default for Look {
    fn default() -> Self {
        Look { theme: Theme::Catalyst, wallpaper: Wallpaper::Catalyst, corners: Corners::Standard, letters: Letters::Crisp, width: Width::Normal }
    }
}

impl Theme {
    /// The wallpaper and accent a theme comes with.
    pub fn defaults(self) -> (Wallpaper, Accent) {
        match self {
            Theme::Catalyst => (Wallpaper::Catalyst, Accent::Rose),
            // Bezel's wall panels: cool neutral greys, no brand glow, one orange.
            Theme::Bezel => (Wallpaper::Plain, Accent::Orange),
        }
    }
}

impl Corners {
    /// What every corner radius is multiplied by.
    pub fn scale(self) -> f32 {
        match self {
            Corners::Round => 1.35,
            Corners::Standard => 1.0,
            Corners::Square => 0.45,
        }
    }
}

impl Letters {
    /// The ROND axis: how round the letters' corners are.
    pub fn round(self) -> f32 {
        match self {
            Letters::Crisp => 0.0,
            Letters::Soft => 50.0,
            Letters::Round => 100.0,
        }
    }
}

impl Width {
    /// What each type role's own width is multiplied by.
    pub fn scale(self) -> f32 {
        match self {
            Width::Narrow => 0.9,
            Width::Normal => 1.0,
            Width::Wide => 1.1,
        }
    }
}

/// A soft round light: 1 at the middle to 0 at `radius`, eased.
fn light(x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> f32 {
    let (dx, dy) = ((x - cx) / rx, (y - cy) / ry);
    let t = (1.0 - (dx * dx + dy * dy).sqrt()).max(0.0);
    t * t * (3.0 - 2.0 * t)
}

/// The wallpaper, `WALL` in size, for a palette.
pub fn wallpaper(kind: Wallpaper, t: &Tokens) -> Vec<u32> {
    let (w, h) = WALL;
    let dark = t.tone == Tone::Dark;
    let mut px = Vec::with_capacity(w * h);
    // Positions are given as fractions of the panel, measured inside the margin.
    let at = |fx: f32, fy: f32| (REACH.0 as f32 + fx * W, REACH.1 as f32 + fy * H);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let color = match kind {
                Wallpaper::Catalyst => {
                    // The logo's ground: a diagonal from lighter at the top left
                    // to deepest at the bottom right, tinted a touch toward the
                    // palette, with a pool of light near the top left.
                    let d = ((fx / w as f32 + fy / h as f32) * 0.5).clamp(0.0, 1.0);
                    let base = if dark {
                        let (a, b, c) = (Color::hex(0x191a20), Color::hex(0x101116), Color::hex(0x08090b));
                        let g = if d < 0.55 { a.mix(b, d / 0.55) } else { b.mix(c, (d - 0.55) / 0.45) };
                        g.mix(t.ground, 0.25)
                    } else {
                        t.surface1.mix(t.ground, d)
                    };
                    let (px0, py0) = at(0.26, 0.20);
                    let pool = light(fx, fy, px0, py0, W * 0.64, H * 0.64 * 1.8);
                    let (sx, sy) = at(0.95, 1.05);
                    let warm = light(fx, fy, sx, sy, W * 0.55, H * 0.9);
                    let lit = blend(base.to_u32(), Color::WHITE, pool * if dark { 0.07 } else { 0.45 });
                    let c = Color::hex(lit);
                    Color::hex(blend(c.to_u32(), t.signal, warm * if dark { 0.06 } else { 0.05 }))
                }
                Wallpaper::Glow => {
                    let (sx, sy) = at(0.125, 0.0);
                    let (ix, iy) = at(0.94, 1.0);
                    let c = blend(t.ground.to_u32(), t.signal, light(fx, fy, sx, sy, 150.0, 150.0) * 0.07);
                    Color::hex(blend(c, t.ice, light(fx, fy, ix, iy, 130.0, 130.0) * 0.05))
                }
                Wallpaper::Aurora => {
                    // Three wide, soft bands of the palette's colours.
                    let (ax, ay) = at(0.15, 0.1);
                    let (bx, by) = at(0.85, 0.15);
                    let (cx, cy) = at(0.5, 1.05);
                    let k = if dark { 1.0 } else { 0.6 };
                    let c = blend(t.ground.to_u32(), t.signal, light(fx, fy, ax, ay, W * 0.55, H * 0.6) * 0.13 * k);
                    let c = blend(c, t.ice, light(fx, fy, bx, by, W * 0.5, H * 0.7) * 0.10 * k);
                    Color::hex(blend(c, t.amber, light(fx, fy, cx, cy, W * 0.7, H * 0.45) * 0.07 * k))
                }
                Wallpaper::Plain => t.ground,
                Wallpaper::Deep => {
                    if dark {
                        Color::BLACK
                    } else {
                        Color::WHITE
                    }
                }
            };
            px.push(color.to_u32());
        }
    }
    px
}

/// Copy the panel's window of the wallpaper, `(ox, oy)` into it, to `dst`.
pub fn window(wall: &[u32], ox: usize, oy: usize, dst: &mut [u32]) {
    let (w, wh) = (W as usize, H as usize);
    let (ox, oy) = (ox.min(2 * REACH.0), oy.min(2 * REACH.1));
    for y in 0..wh {
        let from = (y + oy) * WALL.0 + ox;
        dst[y * w..(y + 1) * w].copy_from_slice(&wall[from..from + w]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_wallpaper_is_drawn_whole_in_both_tones() {
        for &kind in Wallpaper::ALL {
            for tokens in [Tokens::of(Accent::Rose, Tone::Dark), Tokens::of(Accent::Hue(27), Tone::Light)] {
                let wall = wallpaper(kind, &tokens);
                assert_eq!(wall.len(), WALL.0 * WALL.1);
                let mut out = vec![0; (W * H) as usize];
                window(&wall, 2 * REACH.0, 2 * REACH.1, &mut out);
                window(&wall, 999, 999, &mut out);
            }
        }
        assert_eq!(Theme::Catalyst.step(1), Theme::Bezel);
        assert_eq!(Width::Narrow.step(-1), Width::Wide);
        assert_eq!(Wallpaper::from_name("aurora"), Some(Wallpaper::Aurora));
    }
}
