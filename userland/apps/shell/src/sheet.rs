//! A sheet of glass grown out of the island over any screen, as Spotlight's
//! bar is: the clipboard's history (`clipboard/panel.rs`) and quick capture
//! (`capture.rs`). The island stretches into it on Bezel's arriving spring
//! while its glass materializes (its refraction ramps up, never its
//! opacity), the screen behind is held still and dims a little, and its
//! words come once it is past halfway, as the island's do. Put away, it
//! shrinks back on the smooth spring and its glass dematerializes 240 ms
//! later. Its height glides as what it holds changes. Every spring can be
//! turned round mid-flight: brought up again as it goes, it grows back from
//! where it is. Brought up over the other sheet, it takes that one's glass
//! as it is: the height glides to its own and its words fade in.

use bezel::{Align, Color, GlassShape, Rect, TextStyle};
use detent::{role, Animated};

use crate::gfx::{Gfx, H, W};

/// The sheet's place: across the panel, its top on the island's (as Spotlight's).
pub const SIDE: f32 = 6.0;
pub const TOP: f32 = 4.0;
/// Its corners (Spotlight's), and the inset of what sits in it.
pub const RADIUS: f32 = 16.0;
pub const INSET: f32 = 5.0;
/// Its foot stays this far above the panel's.
pub const FOOT: f32 = 4.0;
/// How much the screen behind dims as it comes (Spotlight's: my own tuning).
const DIM: f32 = 0.28;
/// How long after it starts back into the island its glass begins to go
/// (Bezel's island: 240 ms).
const GLASS_LINGERS: f32 = 0.24;

pub struct Grown {
    open: bool,
    /// 0 the island alone, 1 the sheet grown to its whole shape.
    grow: Animated,
    /// Its glass: 0 → 1 as it materializes.
    glass: Animated,
    /// Seconds since it was put away, for its glass to linger as it shrinks.
    since_closed: f32,
    /// Its height at rest, gliding as what it holds changes.
    height: Animated,
    /// Its words, fading in over the glass of the sheet it took over (else 1).
    words_in: Animated,
    pub calm: bool,
}

impl Default for Grown {
    fn default() -> Self {
        Self::new()
    }
}

impl Grown {
    pub fn new() -> Self {
        Grown {
            open: false,
            grow: Animated::new(0.0, role::release()).with_rest(0.002),
            glass: Animated::new(0.0, role::layer()).with_rest(0.004),
            since_closed: f32::MAX,
            height: Animated::new(30.0, role::release()).with_rest(0.2),
            words_in: Animated::new(1.0, role::effect()).with_rest(0.004),
            calm: false,
        }
    }

    /// Taking keys.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// On screen at all, on its way out included.
    pub fn visible(&self) -> bool {
        self.open || self.grow.value() > 0.002 || self.glass.value() > 0.004
    }

    /// How far it has grown out of the island, 0 to 1.
    pub fn shown(&self) -> f32 {
        self.grow.value().clamp(0.0, 1.0)
    }

    /// How much the screen behind is dimmed just now.
    pub fn dim(&self) -> f32 {
        DIM * self.shown()
    }

    /// How far its words have come in: from halfway grown, as the island's.
    pub fn words(&self) -> f32 {
        ((self.grow.value() - 0.55) / 0.45).clamp(0.0, 1.0) * self.words_in.value().clamp(0.0, 1.0)
    }

    /// Whether it was on its way back into the island: brought up again, it
    /// turns round from there, keeping what it held.
    pub fn returning(&self) -> bool {
        !self.open && self.visible()
    }

    /// Bring it up `height` tall: out of the island, turned round from
    /// where it is on its way back, or in place of the sheet `over` (on
    /// screen still), from its glass as it is.
    pub fn summon(&mut self, calm: bool, height: f32, over: Option<&Grown>) {
        let returning = self.returning();
        self.calm = calm;
        self.open = true;
        self.since_closed = f32::MAX;
        self.words_in.jump(1.0);
        if let Some(over) = over.filter(|o| o.visible() && !calm) {
            self.grow.jump(over.grow.value());
            self.glass.jump(over.glass.value());
            self.height.jump(over.height.value());
            self.height.set_with(height, role::release());
            self.words_in.jump(0.0);
            self.words_in.set_with(1.0, role::effect());
        } else if !returning {
            self.height.jump(height);
        }
        if calm {
            self.grow.jump(1.0);
            self.glass.jump(1.0);
        } else {
            self.grow.set_with(1.0, role::release());
            self.glass.set_with(1.0, role::layer());
        }
    }

    /// Put it away: back into the island on the smooth spring.
    pub fn dismiss(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        self.since_closed = 0.0;
        if self.calm {
            self.grow.jump(0.0);
            self.glass.jump(0.0);
        } else {
            self.grow.set_with(0.0, role::smooth());
        }
    }

    /// Gone at once: something opens out of it, or takes the screen.
    pub fn gone_now(&mut self) {
        self.open = false;
        self.grow.jump(0.0);
        self.glass.jump(0.0);
        self.since_closed = f32::MAX;
    }

    /// Its height at rest from now: gliding there, growing on the arriving
    /// spring and shrinking on the smooth one (at once while it is still
    /// growing out of the island, or in the calm version).
    pub fn set_height(&mut self, g: &mut Gfx, height: f32) {
        let height = height.min(H - TOP - FOOT);
        if (self.height.target() - height).abs() < 0.25 {
            return;
        }
        if self.calm || self.grow.value() < 0.05 || !self.open {
            self.height.jump(height);
        } else {
            let growing = height > self.height.target();
            self.height.set_with(height, if growing { role::release() } else { role::smooth() });
            g.started_moving();
        }
    }

    /// Advance its motion; true while anything moves.
    pub fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.grow.tick(dt) | self.glass.tick(dt) | self.height.tick(dt) | self.words_in.tick(dt);
        // The glass lingers as the sheet shrinks, then dematerializes.
        if !self.open && self.since_closed < f32::MAX {
            self.since_closed += dt.max(0.001);
            if self.since_closed >= GLASS_LINGERS && self.glass.target() > 0.0 {
                self.glass.set_with(0.0, role::effect());
            }
            moving = true;
            if !self.visible() {
                self.since_closed = f32::MAX;
            }
        }
        moving
    }

    /// Where it rests: across the panel from the island's top.
    pub fn rest(&self) -> Rect {
        Rect::new(SIDE, TOP, W - 2.0 * SIDE, self.height.value().max(1.0))
    }

    /// Its glass, for the glass pass: grown out of `island` as far as it
    /// has, as materialized as it is. It goes first among the island's
    /// group, so where the two meet it is the sheet's.
    pub fn glass(&self, g: &Gfx, island: Rect) -> GlassShape {
        let t = self.grow.value();
        let rest = self.rest();
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        // Past its rest (the spring runs a little over) it grows a little
        // more, never past the panel's edges.
        let (x, y) = (lerp(island.x, rest.x).max(1.0), lerp(island.y, rest.y).max(1.0));
        let rect = Rect::new(x, y, lerp(island.w, rest.w).min(W - 1.0 - x), lerp(island.h, rest.h).clamp(1.0, H - 2.0 - y));
        let radius = (island.h * 0.5 + (g.radius(RADIUS) - island.h * 0.5) * t.clamp(0.0, 1.0)).min(rect.h * 0.5).min(rect.w * 0.5);
        GlassShape { rect, radius, group: 0, strength: self.glass.value().clamp(0.0, 1.0), tint: None, clear: false }
    }
}

// ---------------------------------------------------------------- words ---

/// Right-aligned text centred on `cy`, at an opacity.
pub fn text_right(g: &mut Gfx, right: f32, cy: f32, s: &str, style: TextStyle, color: Color, alpha: f32) {
    let baseline = (cy + style.size * 0.36).round();
    g.text.draw_aligned(&mut g.canvas, right, baseline, Align::Right, s, &style, color, alpha);
}

/// The end of `text`, as much as fits, an ellipsis before it if some is
/// left out: where the typing is, is what matters.
pub fn fit_tail(g: &mut Gfx, text: &str, style: TextStyle, width: f32) -> String {
    if g.measure(text, style) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for start in 1..chars.len() {
        let tail: String = std::iter::once('…').chain(chars[start..].iter().copied()).collect();
        if g.measure(&tail, style) <= width {
            return tail;
        }
    }
    chars.last().map(|c| c.to_string()).unwrap_or_default()
}

/// The first of `choices` (longest first) that fits `width` whole, so a
/// larger text size leaves something out rather than cutting it.
pub fn first_fitting<'a>(g: &mut Gfx, choices: &[&'a str], style: TextStyle, width: f32) -> &'a str {
    choices.iter().copied().find(|s| g.measure(s, style) <= width).or_else(|| choices.last().copied()).unwrap_or("")
}

/// How far the keys along a sheet's foot have come in, 0 to 1: as its glass
/// grows down over where they rest, not before, so they are never seen cut
/// by its edge.
pub fn keys_shown(glass: Rect, keys_cy: f32, keys_h: f32) -> f32 {
    ((glass.bottom() - (keys_cy + keys_h * 0.5)) / 8.0 + 1.0).clamp(0.0, 1.0)
}

/// Keys along a sheet's foot, each as a quiet keycap and what it does
/// ("enter" "paste"), from `left`; those that do not fit before `right`
/// are left out, from the last (they are given most needed first). Where
/// they end.
pub fn keys_line(g: &mut Gfx, left: f32, right: f32, cy: f32, keys: &[(&str, &str)], alpha: f32) -> f32 {
    let t = g.tokens;
    let style = g.ty.caption;
    let cap_h = (style.size + 4.0).round();
    let widths: Vec<(f32, f32)> = keys.iter().map(|(k, what)| (g.measure(k, style) + 8.0, g.measure(what, style))).collect();
    let group = |&(cap, words): &(f32, f32)| cap + 4.0 + words;
    let mut count = keys.len();
    while count > 0 && widths[..count].iter().map(group).sum::<f32>() + 10.0 * (count - 1) as f32 > right - left {
        count -= 1;
    }
    let mut x = left;
    for (i, (key, what)) in keys.iter().take(count).enumerate() {
        let (cap_w, words_w) = widths[i];
        let cap = Rect::new(x, (cy - cap_h * 0.5).round(), cap_w, cap_h);
        g.canvas.rrect(cap, 3.5_f32.min(cap_h * 0.5), t.ink, 0.10 * alpha);
        g.text_alpha(cap.x + 4.0, cy, key, style, t.ink, alpha);
        g.text_alpha(cap.right() + 4.0, cy, what, style, t.dim, alpha);
        x = cap.right() + 4.0 + words_w + 10.0;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_come_in_only_as_the_glass_reaches_them() {
        // Keys resting 90 to 110 down.
        let glass = |bottom: f32| Rect::new(0.0, 0.0, 100.0, bottom);
        assert_eq!(keys_shown(glass(110.0), 100.0, 20.0), 1.0, "covered: in");
        assert_eq!(keys_shown(glass(150.0), 100.0, 20.0), 1.0);
        assert_eq!(keys_shown(glass(102.0), 100.0, 20.0), 0.0, "8 short: not yet");
        assert!((keys_shown(glass(106.0), 100.0, 20.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_sheet_brought_up_over_the_other_takes_its_glass_as_it_is() {
        let mut history = Grown::new();
        history.summon(false, 150.0, None);
        for _ in 0..90 {
            history.tick(1.0 / 60.0);
        }
        let mut capture = Grown::new();
        capture.summon(false, 80.0, Some(&history));
        assert!(capture.shown() > 0.99, "not grown again out of the island");
        assert!((capture.rest().h - 150.0).abs() < 0.5, "from the history's height");
        assert!(capture.words() < 0.01, "its words still to come");
        for _ in 0..90 {
            capture.tick(1.0 / 60.0);
        }
        assert!((capture.rest().h - 80.0).abs() < 0.5 && capture.words() > 0.99, "its own height, its words in");
        // Calm: at once.
        let mut calm = Grown::new();
        calm.summon(true, 80.0, Some(&history));
        assert_eq!((calm.rest().h, calm.words()), (80.0, 1.0));
    }
}
