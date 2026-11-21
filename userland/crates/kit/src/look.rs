//! The look the handheld is wearing, as the shell hands it to an app: the
//! accent and mode, the text size, the theme, corners, letters and
//! wallpaper, and the calm version. Read once, from the environment the
//! shell starts an app with (`CATALYST_LIGHT`, `CATALYST_ACCENT`,
//! `CATALYST_TEXT`, `CATALYST_THEME` and the rest); run on its own, an app
//! wears Catalyst's own look.

use bezel::{Accent, Color, TextStyle, Tokens, Tone};

use crate::ui::{H, W};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextSize {
    Compact,
    Standard,
    Large,
}

impl TextSize {
    pub const ALL: [TextSize; 3] = [TextSize::Compact, TextSize::Standard, TextSize::Large];

    pub fn name(self) -> &'static str {
        match self {
            TextSize::Compact => "compact",
            TextSize::Standard => "standard",
            TextSize::Large => "large",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name.trim())
    }
}

/// The type roles at one text size: the shell's, so an app's words are the
/// same size as the shell's beside them.
#[derive(Clone, Copy, Debug)]
pub struct Type {
    /// The smallest text: lowercase mono captions and hints.
    pub caption: TextStyle,
    /// Mono values.
    pub label: TextStyle,
    /// Rows and control labels.
    pub body: TextStyle,
    /// Names of things.
    pub name: TextStyle,
    /// Page titles.
    pub title: TextStyle,
    /// Multiplies display (glanceable value) sizes.
    pub display_scale: f32,
}

impl Type {
    pub fn of(size: TextSize) -> Type {
        let (caption, label, body, name, title, display_scale) = match size {
            TextSize::Compact => (10.0, 11.0, 12.0, 13.0, 15.0, 0.9),
            // Nothing under 11 px (about 1.5 mm on this panel).
            TextSize::Standard => (11.0, 12.0, 13.0, 15.0, 16.0, 1.0),
            TextSize::Large => (13.0, 14.0, 15.0, 17.0, 18.0, 1.1),
        };
        Type {
            caption: TextStyle::label(caption),
            label: TextStyle::label(label),
            body: TextStyle::body(body),
            name: TextStyle::name(name),
            title: TextStyle::title(title),
            display_scale,
        }
    }

    /// The display voice, for a glanceable value.
    pub fn display(&self, size: f32) -> TextStyle {
        TextStyle::display(size * self.display_scale)
    }
}

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
                Self::ALL.iter().copied().find(|v| v.name() == name.trim())
            }
        }
    };
}

named!(Theme { Catalyst => "catalyst", Bezel => "bezel" });
named!(Wallpaper { Catalyst => "catalyst", Glow => "glow", Aurora => "aurora", Plain => "plain", Deep => "deep" });
named!(Corners { Round => "round", Standard => "standard", Square => "square" });
named!(Letters { Crisp => "crisp", Soft => "soft", Round => "round" });
named!(Width { Narrow => "narrow", Normal => "normal", Wide => "wide" });
named!(IslandRest { Robot => "robot", Clock => "clock", Battery => "battery" });

/// Everything about how the handheld looks, as an app needs it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub light: bool,
    pub accent: Accent,
    pub text: TextSize,
    /// Reduced transparency and motion: glass drawn solid, no overshoot.
    pub calm: bool,
    pub theme: Theme,
    pub wallpaper: Wallpaper,
    pub corners: Corners,
    pub letters: Letters,
    pub width: Width,
    /// The 24-hour clock.
    pub clock24: bool,
    /// What the island shows when it has nothing to say.
    pub island: IslandRest,
}

impl Default for Look {
    /// Catalyst's own look: dark, rose, the standard text size.
    fn default() -> Self {
        Look {
            light: false,
            accent: Accent::Rose,
            text: TextSize::Standard,
            calm: false,
            theme: Theme::Catalyst,
            wallpaper: Wallpaper::Catalyst,
            corners: Corners::Standard,
            letters: Letters::Crisp,
            width: Width::Normal,
            clock24: true,
            island: IslandRest::Robot,
        }
    }
}

impl Look {
    /// The look the shell started this app with.
    pub fn from_env() -> Look {
        Look::from_vars(|name| std::env::var(name).ok())
    }

    /// The look from whatever `var` gives for each of the shell's variables.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Look {
        let d = Look::default();
        let flag = |name: &str, default: bool| var(name).map_or(default, |v| v.trim() == "1");
        let theme = var("CATALYST_THEME").and_then(|v| Theme::from_name(&v)).unwrap_or(d.theme);
        // A theme brings its own wallpaper unless one is named.
        let wallpaper = var("CATALYST_WALLPAPER").and_then(|v| Wallpaper::from_name(&v)).unwrap_or(match theme {
            Theme::Catalyst => Wallpaper::Catalyst,
            Theme::Bezel => Wallpaper::Plain,
        });
        Look {
            light: flag("CATALYST_LIGHT", d.light),
            accent: var("CATALYST_ACCENT").and_then(|v| Accent::from_name(v.trim())).unwrap_or(match theme {
                Theme::Catalyst => Accent::Rose,
                Theme::Bezel => Accent::Orange,
            }),
            text: var("CATALYST_TEXT").and_then(|v| TextSize::from_name(&v)).unwrap_or(d.text),
            calm: flag("CATALYST_CALM", d.calm),
            theme,
            wallpaper,
            corners: var("CATALYST_CORNERS").and_then(|v| Corners::from_name(&v)).unwrap_or(d.corners),
            letters: var("CATALYST_LETTERS").and_then(|v| Letters::from_name(&v)).unwrap_or(d.letters),
            width: var("CATALYST_WIDTH").and_then(|v| Width::from_name(&v)).unwrap_or(d.width),
            clock24: flag("CATALYST_CLOCK24", d.clock24),
            island: var("CATALYST_ISLAND").and_then(|v| IslandRest::from_name(&v)).unwrap_or(d.island),
        }
    }

    pub fn tokens(&self) -> Tokens {
        Tokens::of(self.accent, if self.light { Tone::Light } else { Tone::Dark })
    }

    /// What every corner radius is multiplied by.
    pub fn corner_scale(&self) -> f32 {
        match self.corners {
            Corners::Round => 1.35,
            Corners::Standard => 1.0,
            Corners::Square => 0.45,
        }
    }

    /// The type roles at this look's text size, with Catalyst's titles a step
    /// larger and a touch heavier, as the shell has them.
    pub fn type_roles(&self) -> Type {
        let mut ty = Type::of(self.text);
        if self.theme == Theme::Catalyst {
            ty.title = TextStyle { size: ty.title.size + 1.0, wght: 650.0, wdth: 100.0, ..ty.title };
        }
        ty
    }

    /// The letters' roundness (the ROND axis) and width multiplier.
    pub fn letter_axes(&self) -> (f32, f32) {
        let round = match self.letters {
            Letters::Crisp => 0.0,
            Letters::Soft => 50.0,
            Letters::Round => 100.0,
        };
        let width = match self.width {
            Width::Narrow => 0.9,
            Width::Normal => 1.0,
            Width::Wide => 1.1,
        };
        (round, width)
    }

    /// The environment that hands this look on (to a program an app starts).
    pub fn vars(&self) -> Vec<(&'static str, String)> {
        vec![
            ("CATALYST_LIGHT", if self.light { "1" } else { "0" }.into()),
            ("CATALYST_ACCENT", self.accent.key()),
            ("CATALYST_TEXT", self.text.name().into()),
            ("CATALYST_CALM", if self.calm { "1" } else { "0" }.into()),
            ("CATALYST_THEME", self.theme.name().into()),
            ("CATALYST_WALLPAPER", self.wallpaper.name().into()),
            ("CATALYST_CORNERS", self.corners.name().into()),
            ("CATALYST_LETTERS", self.letters.name().into()),
            ("CATALYST_WIDTH", self.width.name().into()),
            ("CATALYST_CLOCK24", if self.clock24 { "1" } else { "0" }.into()),
            ("CATALYST_ISLAND", self.island.name().into()),
        ]
    }
}

/// A soft round light: 1 at the middle to 0 at `radius`, eased.
fn light(x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> f32 {
    let (dx, dy) = ((x - cx) / rx, (y - cy) / ry);
    let t = (1.0 - (dx * dx + dy * dy).sqrt()).max(0.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far the shell's wallpaper reaches past the panel, so the panel-sized
/// window here is the one the shell shows behind a page at rest.
const REACH: (f32, f32) = (48.0, 12.0);

/// The wallpaper behind every page, the panel's size: the shell's own, at
/// the place it rests behind an app's page.
pub fn wallpaper(kind: Wallpaper, t: &Tokens) -> Vec<u32> {
    let (w, h) = (W as usize, H as usize);
    let (ww, wh) = (W + 2.0 * REACH.0, H + 2.0 * REACH.1);
    let dark = t.tone == Tone::Dark;
    let at = |fx: f32, fy: f32| (REACH.0 + fx * W, REACH.1 + fy * H);
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            // In the shell's wallpaper's own coordinates.
            let (fx, fy) = (x as f32 + REACH.0 + 0.5, y as f32 + REACH.1 + 0.5);
            let color = match kind {
                Wallpaper::Catalyst => {
                    let d = ((fx / ww + fy / wh) * 0.5).clamp(0.0, 1.0);
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
                    let lit = bezel::canvas::blend(base.to_u32(), Color::WHITE, pool * if dark { 0.07 } else { 0.45 });
                    Color::hex(bezel::canvas::blend(lit, t.signal, warm * if dark { 0.06 } else { 0.05 }))
                }
                Wallpaper::Glow => {
                    let (sx, sy) = at(0.125, 0.0);
                    let (ix, iy) = at(0.94, 1.0);
                    let c = bezel::canvas::blend(t.ground.to_u32(), t.signal, light(fx, fy, sx, sy, 150.0, 150.0) * 0.07);
                    Color::hex(bezel::canvas::blend(c, t.ice, light(fx, fy, ix, iy, 130.0, 130.0) * 0.05))
                }
                Wallpaper::Aurora => {
                    let (ax, ay) = at(0.15, 0.1);
                    let (bx, by) = at(0.85, 0.15);
                    let (cx, cy) = at(0.5, 1.05);
                    let k = if dark { 1.0 } else { 0.6 };
                    let c = bezel::canvas::blend(t.ground.to_u32(), t.signal, light(fx, fy, ax, ay, W * 0.55, H * 0.6) * 0.13 * k);
                    let c = bezel::canvas::blend(c, t.ice, light(fx, fy, bx, by, W * 0.5, H * 0.7) * 0.10 * k);
                    Color::hex(bezel::canvas::blend(c, t.amber, light(fx, fy, cx, cy, W * 0.7, H * 0.45) * 0.07 * k))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_look_is_read_from_what_the_shell_passes() {
        let vars = |pairs: &'static [(&'static str, &'static str)]| move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string());
        let look = Look::from_vars(vars(&[("CATALYST_LIGHT", "1"), ("CATALYST_ACCENT", "hue-215"), ("CATALYST_TEXT", "large"), ("CATALYST_THEME", "bezel")]));
        assert!(look.light);
        assert_eq!(look.accent.key(), "hue-215");
        assert_eq!((look.text, look.theme, look.wallpaper), (TextSize::Large, Theme::Bezel, Wallpaper::Plain));
        // Nothing passed: Catalyst's own look.
        assert_eq!(Look::from_vars(|_| None), Look::default());
        // What it hands on reads back the same.
        let round = Look { calm: true, corners: Corners::Round, letters: Letters::Soft, island: IslandRest::Clock, ..look };
        let passed = round.vars();
        assert_eq!(Look::from_vars(|name| passed.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone())), round);
    }

    #[test]
    fn every_wallpaper_fills_the_panel_in_both_tones() {
        for &kind in Wallpaper::ALL {
            for tokens in [Tokens::of(Accent::Rose, Tone::Dark), Tokens::of(Accent::Hue(27), Tone::Light)] {
                assert_eq!(wallpaper(kind, &tokens).len(), (W * H) as usize);
            }
        }
    }
}
