//! What a page draws with: Bezel's surfaces, type and icons on the 320 × 170
//! panel, measured as the shell measures them, and the controls every page
//! shares (rows, switches, buttons, chips, meters, the hint line), with the
//! focus ring that glides between them.
//!
//! Lay pages out from the type, never in fixed pixels: [`Ui::top`] is where
//! a page starts under the island, [`Ui::row_h`] a row's height,
//! [`Ui::hint_h`] the hint line's. Then the large text size fits as well as
//! the standard one.

use bezel::{Align, Canvas, Color, Glass, GlassShape, Icons, Rect, Text, TextStyle, Tokens};
use detent::{role, Animated, Spring};

use crate::look::{Look, Theme, Type};

pub const W: f32 = 320.0;
pub const H: f32 = 170.0;
/// Page padding and the gap between tiles (Bezel's 30 and 14, scaled).
pub const PAD: f32 = 8.0;
pub const GAP: f32 = 5.0;
/// A tile's or card's corner radius (Bezel's 32, scaled).
pub const TILE_RADIUS: f32 = 14.0;

/// Pages rising in: an arriving page's rows come up this far into place,
/// one after another this far apart (the shell's own tuning).
const RISE: f32 = 10.0;
const STAGGER: f32 = 0.035;
pub(crate) const RISE_LAST: f32 = 8.0;
const REVEAL_OVER: f32 = RISE_LAST * STAGGER + 0.55;

/// Material 3 Expressive's spatial springs, as Detent carries them and the
/// shell uses them: for things that move under the keys. The calm version
/// damps them critically.
pub mod spatial {
    use detent::Spring;

    /// `m3-expressive-fast-spatial` (800, 0.6): small moves with a visible bounce.
    pub fn fast() -> Spring {
        Spring::stiffness_ratio(800.0, 0.6)
    }

    /// `m3-expressive-default-spatial` (380, 0.8): the everyday move.
    pub fn default() -> Spring {
        Spring::stiffness_ratio(380.0, 0.8)
    }

    /// `m3-expressive-slow-spatial` (200, 0.8): a whole page arriving.
    pub fn slow() -> Spring {
        Spring::stiffness_ratio(200.0, 0.8)
    }
}

/// Moving, the ring stretches along its motion and its tail trails, as
/// Bezel's dock droplet does (the shell's numbers).
const RING_STRETCH: f32 = 0.04;
const RING_STRETCH_MAX: f32 = 30.0 * W / 720.0;

fn stretched(r: Rect, vx: f32, vy: f32) -> Rect {
    let along = |v: f32, size: f32| (v.abs() * RING_STRETCH).min(RING_STRETCH_MAX).min(size / 3.0);
    let (sx, sy) = (along(vx, r.w), along(vy, r.h));
    let (thin_w, thin_h) = ((sy * 0.5).min(r.w * 0.08), (sx * 0.5).min(r.h * 0.08));
    let x = if vx > 0.0 { r.x - sx } else { r.x } + thin_w * 0.5;
    let y = if vy > 0.0 { r.y - sy } else { r.y } + thin_h * 0.5;
    Rect::new(x, y, r.w + sx - thin_w, r.h + sy - thin_h)
}

/// The focus ring's place, size and corners, each on a spring.
struct Focus {
    x: Animated,
    y: Animated,
    w: Animated,
    h: Animated,
    radius: Animated,
    placed: bool,
}

impl Focus {
    fn new() -> Self {
        let at = |spring: Spring| Animated::new(0.0, spring).with_rest(0.05);
        Focus { x: at(spatial::fast()), y: at(spatial::fast()), w: at(spatial::default()), h: at(spatial::default()), radius: at(spatial::default()), placed: false }
    }

    fn springs(&mut self) -> [&mut Animated; 5] {
        [&mut self.x, &mut self.y, &mut self.w, &mut self.h, &mut self.radius]
    }

    fn velocity(&self) -> (f32, f32) {
        (self.x.velocity() + 0.5 * self.w.velocity(), self.y.velocity() + 0.5 * self.h.velocity())
    }
}

fn baseline_for(style: &TextStyle, cy: f32) -> f32 {
    (cy + style.size * 0.36).round()
}

pub struct Ui {
    pub canvas: Canvas,
    pub text: Text,
    pub icons: Icons,
    pub glass: Glass,
    pub tokens: Tokens,
    pub look: Look,
    /// The type roles at the text size in use.
    pub ty: Type,
    /// Sideways offset of what is drawn: pages slide during a turn.
    pub ox: f32,
    /// The width a page title may take before it meets the island.
    pub title_room: f32,
    missing_icons: std::collections::BTreeSet<String>,
    focus: Focus,
    pub(crate) last_ring: Rect,
    /// Draw the ring where asked and leave its glide alone: set while a page
    /// is drawn as a picture for a transition.
    pub(crate) focus_static: bool,
    motion_started: bool,
    reveal: Option<f32>,
    press: Animated,
    scroll: Animated,
    scroll_placed: bool,
    ring_shift: f32,
}

impl Ui {
    pub fn new(look: Look) -> Self {
        let mut text = Text::new();
        let (round, width) = look.letter_axes();
        text.set_letters(round, width);
        Ui {
            canvas: Canvas::new(W as i32, H as i32),
            text,
            icons: Icons::new(),
            glass: Glass::new(),
            tokens: look.tokens(),
            look,
            ty: look.type_roles(),
            ox: 0.0,
            title_room: 96.0,
            missing_icons: Default::default(),
            focus: Focus::new(),
            last_ring: Rect::default(),
            focus_static: false,
            motion_started: false,
            reveal: None,
            press: Animated::new(0.0, role::settle()).with_rest(0.01),
            scroll: Animated::new(0.0, role::hold()).with_rest(0.004),
            scroll_placed: false,
            ring_shift: 0.0,
        }
    }

    /// Whether reduced motion and transparency are asked for.
    pub fn calm(&self) -> bool {
        self.look.calm
    }

    // ---- layout, measured from the type ----

    /// The glass island's height.
    pub fn island_h(&self) -> f32 {
        self.ty.caption.size + 8.0
    }
    /// Where a page's content starts, below the island and title.
    pub fn top(&self) -> f32 {
        self.island_h() + 9.0
    }
    /// A one-line row's height.
    pub fn row_h(&self) -> f32 {
        self.ty.body.size + 10.0
    }
    /// The room the hint line takes at the bottom of a page.
    pub fn hint_h(&self) -> f32 {
        self.ty.caption.size + 4.0
    }
    /// Where a page's content must end, above the hint line.
    pub fn bottom(&self) -> f32 {
        H - PAD - self.hint_h() + 4.0
    }

    /// A corner radius as the corner setting makes it.
    pub fn radius(&self, base: f32) -> f32 {
        base * self.look.corner_scale()
    }
    /// The radius of a tile or a card.
    pub fn tile_radius(&self) -> f32 {
        self.radius(TILE_RADIUS)
    }

    // ---- colours with jobs ----

    /// A fault: the signal where it is already red or orange, otherwise a
    /// fixed red that reads on every surface (the shell's).
    pub fn fault(&self) -> Color {
        let s = self.tokens.signal;
        let (r, g, b) = (s.0 as f32, s.1 as f32, s.2 as f32);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let warm = max == r && max - min > 40.0 && {
            let hue = 60.0 * ((g - b) / (max - min)).rem_euclid(6.0);
            !(45.0..330.0).contains(&hue)
        };
        match (warm, self.tokens.tone) {
            (true, _) => s,
            (false, bezel::Tone::Dark) => Color::hex(0xff6b5e),
            (false, bezel::Tone::Light) => Color::hex(0xb3261e),
        }
    }

    /// Words that read as "fine": ice on dark, ice's own ink on light.
    pub fn good(&self) -> Color {
        match self.tokens.tone {
            bezel::Tone::Dark => self.tokens.ice,
            bezel::Tone::Light => self.tokens.on_ice,
        }
    }

    /// The focus ring's colour: the ink in Catalyst's theme, the signal in Bezel's.
    pub fn ring_color(&self) -> Color {
        match self.look.theme {
            Theme::Catalyst => self.tokens.ink,
            Theme::Bezel => self.tokens.signal,
        }
    }

    // ---- drawing ----

    pub fn clear(&mut self, background: &[u32]) {
        if background.len() == self.canvas.px.len() {
            self.canvas.px.copy_from_slice(background);
        } else {
            self.canvas.clear(self.tokens.ground);
        }
    }

    pub fn surface(&mut self, r: Rect, radius: f32, color: Color) {
        self.canvas.rrect(r.offset(self.ox, 0.0), radius, color, 1.0);
    }

    /// A surface at an opacity, for something fading in or out.
    pub fn surface_alpha(&mut self, r: Rect, radius: f32, color: Color, alpha: f32) {
        self.canvas.rrect(r.offset(self.ox, 0.0), radius, color, alpha.clamp(0.0, 1.0));
    }

    /// Text placed by its left, centre or right on `cy`, at an opacity.
    #[allow(clippy::too_many_arguments)]
    pub fn text_alpha_at(&mut self, anchor: f32, cy: f32, align: Align, s: &str, style: TextStyle, color: Color, alpha: f32) -> f32 {
        self.text.draw_aligned(&mut self.canvas, anchor + self.ox, baseline_for(&style, cy), align, s, &style, color, alpha.clamp(0.0, 1.0))
    }

    /// The first of `choices` (longest first) that fits `width`, centred in
    /// it on `cy`: a hint for part of a page (beside a board).
    pub fn hint_in(&mut self, x: f32, width: f32, cy: f32, choices: &[&str]) {
        let (style, color) = (self.ty.caption, self.tokens.dim);
        let line = choices.iter().copied().find(|c| self.measure(c, style) <= width).or(choices.last().copied()).unwrap_or("");
        let line = self.text.fit(line, &style, width);
        self.text_at(x + width * 0.5, cy, Align::Center, &line, style, color);
    }

    /// Text from its left edge on a baseline. Returns its width.
    pub fn text(&mut self, x: f32, baseline: f32, s: &str, style: TextStyle, color: Color) -> f32 {
        self.text.draw(&mut self.canvas, x + self.ox, baseline, s, &style, color, 1.0)
    }

    /// Text placed by its left, centre or right, centred vertically on `cy`.
    pub fn text_at(&mut self, anchor: f32, cy: f32, align: Align, s: &str, style: TextStyle, color: Color) -> f32 {
        self.text.draw_aligned(&mut self.canvas, anchor + self.ox, baseline_for(&style, cy), align, s, &style, color, 1.0)
    }

    /// Left-aligned text centred on `cy`, cut with an ellipsis to fit.
    pub fn text_fit(&mut self, x: f32, cy: f32, s: &str, style: TextStyle, color: Color, max_width: f32) -> f32 {
        let fitted = self.text.fit(s, &style, max_width.max(0.0));
        self.text_at(x, cy, Align::Left, &fitted, style, color)
    }

    /// Left-aligned text centred on `cy`, at an opacity.
    pub fn text_alpha(&mut self, x: f32, cy: f32, s: &str, style: TextStyle, color: Color, alpha: f32) {
        self.text.draw(&mut self.canvas, x + self.ox, baseline_for(&style, cy), s, &style, color, alpha);
    }

    pub fn measure(&mut self, s: &str, style: TextStyle) -> f32 {
        self.text.measure(s, &style)
    }

    /// `s` broken into lines no wider than `width`, at word boundaries (a
    /// word too long for a line is cut with an ellipsis). Line breaks in
    /// `s` are kept.
    pub fn wrap(&mut self, s: &str, style: TextStyle, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for para in s.split('\n') {
            let mut line = String::new();
            for word in para.split_whitespace() {
                let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
                if self.measure(&candidate, style) <= width {
                    line = candidate;
                } else {
                    if !line.is_empty() {
                        lines.push(std::mem::take(&mut line));
                    }
                    line = if self.measure(word, style) <= width { word.to_owned() } else { self.text.fit(word, &style, width) };
                }
            }
            lines.push(line);
        }
        lines
    }

    /// Words wrapped into `width` from `y` down, at most `max_lines` lines
    /// (the last cut with an ellipsis if there is more). Returns how far
    /// down it went.
    #[allow(clippy::too_many_arguments)]
    pub fn paragraph(&mut self, x: f32, y: f32, width: f32, s: &str, style: TextStyle, color: Color, max_lines: usize) -> f32 {
        let mut lines = self.wrap(s, style, width);
        if lines.len() > max_lines {
            lines.truncate(max_lines);
            if let Some(last) = lines.last_mut() {
                let more = format!("{last}…");
                *last = self.text.fit(&more, &style, width);
            }
        }
        let pitch = (style.size * 1.32).round();
        for (i, line) in lines.iter().enumerate() {
            self.text_at(x, y + pitch * (i as f32 + 0.5), Align::Left, line, style, color);
        }
        pitch * lines.len() as f32
    }

    pub fn icon(&mut self, name: &str, filled: bool, cx: f32, cy: f32, size: f32, color: Color) {
        self.icon_alpha(name, filled, cx, cy, size, color, 1.0);
    }

    /// An icon at an opacity. Material Symbols Rounded by name (the shell's
    /// set), or one an app registered with [`bezel::icons::register`].
    #[allow(clippy::too_many_arguments)]
    pub fn icon_alpha(&mut self, name: &str, filled: bool, cx: f32, cy: f32, size: f32, color: Color, alpha: f32) {
        let drawn = self.icons.draw(&mut self.canvas, name, filled, cx + self.ox, cy, size, color, alpha);
        if !drawn && !name.is_empty() && self.missing_icons.insert(name.to_owned()) {
            eprintln!("catalyst-kit: no icon called {name:?}");
        }
    }

    /// The icon names asked for that are not built in (a test fails on any).
    pub fn missing_icons(&self) -> &std::collections::BTreeSet<String> {
        &self.missing_icons
    }

    pub fn dot(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        self.canvas.circle(cx + self.ox, cy, radius, color, 1.0);
    }

    /// A soft shadow under something lifted, lit from above.
    pub fn shadow(&mut self, r: Rect, radius: f32, strength: f32) {
        if strength <= 0.01 {
            return;
        }
        let base = if self.tokens.tone == bezel::Tone::Dark { 0.34 } else { 0.10 };
        for i in 0..3 {
            let k = i as f32;
            self.canvas.rrect(r.offset(self.ox, 1.5 + k).inset(-k), radius + k, Color::BLACK, base * strength * 0.5f32.powi(i));
        }
    }

    // ---- the focus ring ----

    /// The focus ring, just outside the focused thing. It glides from the
    /// last thing focused with a little overshoot, stretching on its way;
    /// its size and corners follow. In the calm version it lands at once.
    pub fn focus_ring(&mut self, r: Rect, radius: f32) {
        let color = self.ring_color();
        let r = r.offset(0.0, self.ring_shift);
        let (at, radius) = if self.focus_static {
            (r, radius)
        } else {
            let want = [r.x, r.y, r.w, r.h, radius];
            let calm = self.look.calm;
            let f = &mut self.focus;
            if !f.placed || calm {
                for (spring, to) in f.springs().into_iter().zip(want) {
                    spring.jump(to);
                }
                f.placed = true;
            } else if f.springs().iter().zip(want).any(|(spring, to)| spring.target() != to) {
                for (spring, to) in f.springs().into_iter().zip(want) {
                    spring.set(to);
                }
                self.motion_started = true;
            }
            let (vx, vy) = f.velocity();
            let at = Rect::new(f.x.value(), f.y.value(), f.w.value(), f.h.value());
            (if calm { at } else { stretched(at, vx, vy) }, f.radius.value())
        };
        let ring = at.inset(-2.5);
        let radius = (radius + 2.5).min(ring.w * 0.5).min(ring.h * 0.5).max(0.0);
        self.last_ring = ring;
        self.canvas.stroke_rrect(ring.offset(self.ox, 0.0), radius, 1.5, color, 1.0);
    }

    /// Forget where the ring was: the next page's focus is placed at once.
    pub fn focus_reset(&mut self) {
        self.focus.placed = false;
        self.scroll_placed = false;
    }

    /// How lifted a row is: 1 under the ring, falling to 0 a row away.
    fn lift_under_ring(&self, r: Rect, focused: bool) -> f32 {
        let f = &self.focus;
        if self.focus_static || self.look.calm || !f.placed {
            return if focused { 1.0 } else { 0.0 };
        }
        let (cx, cy) = (f.x.value() + f.w.value() * 0.5, f.y.value() + f.h.value() * 0.5);
        if (cx - r.cx()).abs() > r.w * 0.5 {
            return 0.0;
        }
        (1.0 - (cy - r.cy()).abs() / r.h).clamp(0.0, 1.0)
    }

    /// Enter landed on what has the focus: a state layer and a small squash
    /// on it, springing back.
    pub fn pressed(&mut self) {
        if !self.look.calm {
            self.press.jump(1.0);
            self.press.set(0.0);
            self.motion_started = true;
        }
    }

    /// How pressed the focused thing is: 1 as the key lands, 0 at rest.
    pub fn press(&self) -> f32 {
        self.press.value().clamp(0.0, 1.0)
    }

    /// Drawing set something moving: frames should follow at once.
    pub fn started_moving(&mut self) {
        self.motion_started = true;
    }

    pub(crate) fn take_motion_started(&mut self) -> bool {
        std::mem::take(&mut self.motion_started)
    }

    /// Advance what moves on its own. True while something still is.
    pub(crate) fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.scroll.tick(dt) | self.press.tick(dt);
        for spring in self.focus.springs() {
            moving |= spring.tick(dt);
        }
        if let Some(t) = &mut self.reveal {
            *t += dt;
            if *t > REVEAL_OVER {
                self.reveal = None;
            }
            moving = true;
        }
        moving
    }

    // ---- pages rising in ----

    pub(crate) fn reveal_start(&mut self) {
        self.reveal = (!self.look.calm).then_some(0.0);
    }

    pub(crate) fn revealing(&self) -> bool {
        self.reveal.is_some()
    }

    /// How far below its place block `order` of an arriving page still is,
    /// or `None` before its turn.
    pub(crate) fn rise(&self, order: f32) -> Option<f32> {
        let t = match self.reveal {
            Some(t) if !self.look.calm => t,
            _ => return Some(0.0),
        };
        let since = t - order * STAGGER;
        (since >= 0.0).then(|| RISE * role::release().solve(1.0, 0.0, since).0)
    }

    // ---- controls ----

    /// One row of a list: a name on the left, a mono value on the right.
    /// The row under the focus ring lifts, and squashes a little on Enter.
    pub fn row(&mut self, r: Rect, name: &str, value: &str, value_color: Color, focused: bool) {
        let (t, ty) = (self.tokens, self.ty);
        let bezel = self.look.theme == Theme::Bezel;
        let radius = self.row_radius(r);
        self.row_surface(r, radius, focused);
        if bezel {
            let value_w = self.measure(value, ty.body);
            self.text_fit(r.x + 10.0, r.cy(), &name.to_lowercase(), ty.label, t.ink, r.w - 30.0 - value_w);
            self.text_at(r.right() - 10.0, r.cy(), Align::Right, value, ty.body, value_color);
        } else {
            let value_w = self.measure(value, ty.label);
            self.text_fit(r.x + 10.0, r.cy(), name, ty.body, t.ink, r.w - 30.0 - value_w);
            self.text_at(r.right() - 10.0, r.cy(), Align::Right, value, ty.label, value_color);
        }
        if focused {
            self.focus_ring(r, radius);
        }
    }

    /// A row with an icon before its name.
    pub fn icon_row(&mut self, r: Rect, icon: &str, name: &str, value: &str, focused: bool) {
        let (t, ty) = (self.tokens, self.ty);
        let radius = self.row_radius(r);
        self.row_surface(r, radius, focused);
        let size = (r.h - 8.0).clamp(12.0, 18.0);
        self.icon(icon, focused, r.x + 8.0 + size * 0.5, r.cy(), size, if focused { t.ink } else { t.dim });
        let value_w = if value.is_empty() { 0.0 } else { self.text_at(r.right() - 10.0, r.cy(), Align::Right, value, ty.label, t.dim) };
        self.text_fit(r.x + 14.0 + size, r.cy(), name, ty.body, t.ink, r.w - 30.0 - size - value_w);
        if focused {
            self.focus_ring(r, radius);
        }
    }

    /// A row with a switch at its end: on is filled with ice.
    pub fn switch_row(&mut self, r: Rect, name: &str, on: bool, focused: bool) {
        let (t, ty) = (self.tokens, self.ty);
        let radius = self.row_radius(r);
        self.row_surface(r, radius, focused);
        let (tw, th) = (28.0, 15.0);
        let track = Rect::new(r.right() - 10.0 - tw, r.cy() - th * 0.5, tw, th);
        self.surface(track, th * 0.5, if on { t.ice } else { t.track });
        let knob_x = if on { track.right() - th * 0.5 } else { track.x + th * 0.5 };
        self.dot(knob_x, track.cy(), th * 0.5 - 2.5, if on { t.on_ice } else { t.dim });
        let label = if self.look.theme == Theme::Bezel { name.to_lowercase() } else { name.to_owned() };
        let style = if self.look.theme == Theme::Bezel { ty.label } else { ty.body };
        self.text_fit(r.x + 10.0, r.cy(), &label, style, t.ink, r.w - 30.0 - tw);
        if focused {
            self.focus_ring(r, radius);
        }
    }

    fn row_radius(&self, r: Rect) -> f32 {
        if self.look.theme == Theme::Bezel {
            self.radius(7.0).min(r.h * 0.5)
        } else {
            r.h * 0.5
        }
    }

    /// A row's surface: lifted under the focus ring, pressed on Enter.
    fn row_surface(&mut self, r: Rect, radius: f32, focused: bool) {
        let t = self.tokens;
        let lift = self.lift_under_ring(r, focused);
        let press = if focused { self.press() } else { 0.0 };
        let lifted = r.inset(-1.0 * lift + 1.5 * press);
        let corner = (radius + lift - 1.5 * press).max(0.0);
        self.shadow(lifted, corner, lift);
        let fill = t.surface1.mix(t.surface2, lift).mix(t.ink, 0.2 * press);
        self.surface(lifted, corner, fill);
    }

    /// A button: a tonal capsule with an optional icon. `primary` marks the
    /// page's main action in the signal colour (on its icon and word, never
    /// as a fill under small text).
    pub fn button(&mut self, r: Rect, icon: &str, label: &str, focused: bool, primary: bool) {
        let (t, ty) = (self.tokens, self.ty);
        let radius = r.h * 0.5;
        self.row_surface(r, radius, focused);
        let ink = if primary { t.signal } else { t.ink };
        let size = (r.h - 10.0).clamp(12.0, 18.0);
        let label_w = self.measure(label, ty.name);
        let icon_w = if icon.is_empty() { 0.0 } else { size + 5.0 };
        let total = (icon_w + label_w).min(r.w - 16.0);
        let x = r.cx() - total * 0.5;
        if !icon.is_empty() {
            self.icon(icon, true, x + size * 0.5, r.cy(), size, ink);
        }
        self.text_fit(x + icon_w, r.cy(), label, ty.name, if primary { ink } else { t.ink }, r.w - 16.0 - icon_w);
        if focused {
            self.focus_ring(r, radius);
        }
    }

    /// A small capsule of quiet words ("3 left", "hard"), from `x` centred
    /// on `cy`. Returns its width.
    pub fn chip(&mut self, x: f32, cy: f32, label: &str, fill: Color, ink: Color) -> f32 {
        let style = self.ty.caption;
        let h = style.size + 6.0;
        let w = self.measure(label, style) + 12.0;
        self.surface(Rect::new(x, cy - h * 0.5, w, h), h * 0.5, fill);
        self.text_at(x + 6.0, cy, Align::Left, label, style, ink);
        w
    }

    /// A key as it is printed, for help lines: a small keycap.
    pub fn keycap(&mut self, x: f32, cy: f32, label: &str) -> f32 {
        let (t, style) = (self.tokens, self.ty.caption);
        let h = style.size + 5.0;
        let w = (self.measure(label, style) + 8.0).max(h);
        let r = Rect::new(x, cy - h * 0.5, w, h);
        self.surface(r.offset(0.0, 1.0), self.radius(4.0), t.surface3);
        self.surface(r, self.radius(4.0), t.surface2);
        self.text_at(r.cx(), cy, Align::Center, label, style, t.ink);
        w
    }

    /// A meter: the fill takes the track's outer corners.
    pub fn meter(&mut self, r: Rect, fraction: f32, fill: Color) {
        let track = r.offset(self.ox, 0.0);
        let radius = r.h * 0.5;
        self.canvas.rrect(track, radius, self.tokens.meter, 1.0);
        let fraction = fraction.clamp(0.0, 1.0);
        if fraction > 0.0 {
            let w = (track.w * fraction).max(r.h * 0.6);
            self.canvas.rrect_clipped(Rect::new(track.x, track.y, w, track.h), 2.0, track, radius, fill, 1.0);
        }
    }

    /// A tile holding one glanceable value: a quiet label above, the value
    /// in the display voice, and its unit. `None` shows a dash, never 0.
    pub fn stat(&mut self, r: Rect, label: &str, value: Option<&str>, unit: &str, size: f32) {
        let (t, ty) = (self.tokens, self.ty);
        let radius = self.tile_radius();
        self.surface(r, radius, t.surface1);
        self.text_fit(r.x + 9.0, r.y + 5.0 + ty.caption.size * 0.5, label, ty.caption, t.dim, r.w - 18.0);
        let display = ty.display(size);
        let baseline = r.bottom() - 8.0;
        match value {
            Some(v) => {
                let w = self.text(r.x + 8.0, baseline, v, display, t.ink);
                self.text(r.x + 10.0 + w, baseline, unit, ty.caption, t.dim);
            }
            None => {
                self.text(r.x + 8.0, baseline, "–", display, t.faint);
            }
        }
    }

    /// The line along the bottom: what the keys do. Always whole on the
    /// panel; a long one is cut with an ellipsis.
    pub fn hint(&mut self, s: &str) {
        let (style, color) = (self.ty.caption, self.tokens.dim);
        let fitted = self.text.fit(s, &style, W - 2.0 * PAD);
        self.text_at(W * 0.5, H - 2.0 - style.size * 0.5, Align::Center, &fitted, style, color);
    }

    /// The first of `choices` (longest first) that fits whole, so a larger
    /// text size leaves keys out rather than cutting one in half.
    pub fn hint_any(&mut self, choices: &[&str]) {
        let style = self.ty.caption;
        let fits = choices.iter().copied().find(|s| self.measure(s, style) <= W - 2.0 * PAD);
        if let Some(s) = fits.or(choices.last().copied()) {
            self.hint(s);
        }
    }

    /// What a page says when it has nothing to show.
    pub fn empty(&mut self, icon: &str, line: &str) {
        let t = self.tokens;
        let cy = (self.top() + H) * 0.5;
        self.icon(icon, false, W * 0.5, cy - 12.0, 24.0, t.faint);
        let style = self.ty.caption;
        let line = self.text.fit(line, &style, W - 2.0 * PAD);
        self.text_at(W * 0.5, cy + 12.0, Align::Center, &line, style, t.dim);
    }

    /// Run the glass pass over everything drawn so far.
    pub fn glass(&mut self, shapes: &[GlassShape]) {
        self.glass.render(&mut self.canvas, shapes, &self.tokens, bezel::LIGHT_REST, self.look.calm);
    }

    // ---- lists ----

    fn list_scroll(&mut self, first: f32, bump: f32) -> f32 {
        if self.focus_static {
            return first;
        }
        if !self.scroll_placed || self.look.calm {
            self.scroll.jump(first);
            self.scroll_placed = true;
        } else if self.scroll.target() != first {
            self.scroll.set(first);
            self.motion_started = true;
        }
        if bump != 0.0 && !self.look.calm {
            self.scroll.nudge(bump * 22.0);
            self.motion_started = true;
        }
        self.scroll.value()
    }
}

/// A focus over a scrolling list of rows of one height. It glides to where
/// it is scrolled, and gives a little at its ends.
#[derive(Default)]
pub struct List {
    pub focus: usize,
    first: usize,
    visible: usize,
    bump: f32,
    shown: f32,
    placed: bool,
}

impl List {
    pub fn up(&mut self) {
        if self.focus == 0 {
            self.bump = -1.0;
        }
        self.focus = self.focus.saturating_sub(1);
    }

    pub fn down(&mut self, len: usize) {
        if self.focus + 1 < len {
            self.focus += 1;
        } else {
            self.bump = 1.0;
        }
    }

    /// Move by a page: as many rows as were last on screen.
    pub fn page(&mut self, len: usize, direction: isize) {
        let step = self.visible.max(1) as isize * direction;
        self.focus = (self.focus as isize + step).clamp(0, len.saturating_sub(1) as isize) as usize;
    }

    /// Up, Down, the page keys, Home and End. True if the key was one of them.
    pub fn key(&mut self, key: crate::Key, len: usize) -> bool {
        use crate::Key;
        match key {
            Key::Up => self.up(),
            Key::Down => self.down(len),
            Key::PageUp => self.page(len, -1),
            Key::PageDown => self.page(len, 1),
            Key::Home => self.focus = 0,
            Key::End => self.focus = len.saturating_sub(1),
            _ => return false,
        }
        true
    }

    /// The rows to draw, as (index, y), keeping the focus on screen. Drawing
    /// is clipped to the list until [`List::end`], which every list calls last.
    pub fn layout(&mut self, ui: &mut Ui, len: usize, top: f32, row_h: f32, gap: f32, bottom: f32) -> Vec<(usize, f32)> {
        self.focus = self.focus.min(len.saturating_sub(1));
        self.visible = (((bottom - top) + gap) / (row_h + gap)).floor().max(1.0) as usize;
        if self.focus < self.first {
            self.first = self.focus;
        } else if self.focus >= self.first + self.visible {
            self.first = self.focus + 1 - self.visible;
        }
        self.first = self.first.min(len.saturating_sub(self.visible));
        let pitch = row_h + gap;
        if !self.placed && !ui.focus_static {
            ui.scroll_placed = false;
            self.placed = true;
        }
        let scroll = ui.list_scroll(self.first as f32, std::mem::take(&mut self.bump));
        self.shown = scroll;
        ui.ring_shift = (scroll - self.first as f32) * pitch;
        ui.canvas.set_clip(Rect::new(0.0, top - 4.0, W, self.visible as f32 * pitch - gap + 8.0));
        let from = scroll.max(0.0).floor() as usize;
        let to = (scroll.max(0.0).ceil() as usize + self.visible).min(len);
        (from..to).map(|i| (i, top + (i as f32 - scroll) * pitch)).collect()
    }

    /// Ends the clipping `layout` began, with a thin position marker when
    /// the list is longer than the page.
    pub fn end(&self, ui: &mut Ui, len: usize, top: f32, bottom: f32) {
        ui.canvas.clear_clip();
        ui.ring_shift = 0.0;
        if len <= self.visible {
            return;
        }
        let track = bottom - top;
        let thumb = (track * self.visible as f32 / len as f32).max(10.0);
        let y = top + (track - thumb) * (self.shown / (len - self.visible) as f32).clamp(0.0, 1.0);
        let faint = ui.tokens.faint;
        ui.surface(Rect::new(W - 5.0, y, 2.5, thumb), 1.25, faint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 40.0;

    fn row(i: usize) -> Rect {
        Rect::new(10.0, 30.0 + 20.0 * i as f32, 280.0, 18.0)
    }

    #[test]
    fn the_ring_glides_a_little_past_and_rests_exactly() {
        let mut ui = Ui::new(Look::default());
        ui.focus_ring(row(0), 6.0);
        assert_eq!(ui.last_ring, row(0).inset(-2.5), "its first place is taken at once");
        ui.focus_ring(row(1), 6.0);
        assert!(ui.take_motion_started());
        let mut furthest = f32::MIN;
        let mut frames = 0;
        while ui.tick(FRAME) {
            ui.focus_ring(row(1), 6.0);
            furthest = furthest.max(ui.last_ring.bottom());
            frames += 1;
            assert!(frames < 80);
        }
        ui.focus_ring(row(1), 6.0);
        let rest = row(1).inset(-2.5);
        assert!(furthest > rest.bottom() + 1.0, "it runs past: {furthest}");
        assert_eq!(ui.last_ring, rest);
    }

    #[test]
    fn the_calm_ring_lands_at_once() {
        let mut ui = Ui::new(Look { calm: true, ..Look::default() });
        ui.focus_ring(row(0), 6.0);
        ui.focus_ring(row(3), 6.0);
        assert_eq!(ui.last_ring, row(3).inset(-2.5));
        assert!(!ui.tick(FRAME));
    }

    #[test]
    fn words_wrap_at_spaces_and_keep_their_breaks() {
        let mut ui = Ui::new(Look::default());
        let style = ui.ty.body;
        let lines = ui.wrap("the quick brown fox jumps over the lazy dog\nagain", style, 90.0);
        assert!(lines.len() >= 3, "{lines:?}");
        assert_eq!(lines.last().map(String::as_str), Some("again"));
        assert!(lines.iter().all(|l| ui.measure(l, style) <= 90.0));
    }

    #[test]
    fn a_list_keeps_its_focus_in_view() {
        let mut ui = Ui::new(Look { calm: true, ..Look::default() });
        let mut list = List::default();
        for _ in 0..12 {
            list.down(20);
        }
        let rows = list.layout(&mut ui, 20, 30.0, 18.0, 2.0, 110.0);
        list.end(&mut ui, 20, 30.0, 110.0);
        assert!(rows.iter().any(|(i, _)| *i == 12));
        assert!(list.key(crate::Key::End, 20) && list.focus == 19);
        assert!(!list.key(crate::Key::Enter, 20));
    }
}
