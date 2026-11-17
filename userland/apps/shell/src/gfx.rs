//! What screens draw with: Bezel's surfaces, type and icons on the 320 × 170
//! panel, plus the few controls every screen shares.
//!
//! The panel is 1.9 inches, about 190 pixels per inch, so type cannot be
//! Bezel's 720-panel sizes scaled down (a 15 px label would be 6.7 px, under
//! a millimetre). Shapes scale by 320 / 720; type has its own scale with a
//! floor, chosen in System, and every layout is measured from it.

use bezel::{Align, Canvas, Color, Glass, GlassShape, Icons, Rect, Text, TextStyle, Tokens};

pub const W: f32 = 320.0;
pub const H: f32 = 170.0;
/// Page padding and the gap between tiles (Bezel's 30 and 14, scaled).
pub const PAD: f32 = 8.0;
pub const GAP: f32 = 5.0;
/// Tile corner radius (Bezel's 32, scaled).
pub const TILE_RADIUS: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextSize {
    Compact,
    Standard,
    Large,
}

impl TextSize {
    pub fn name(self) -> &'static str {
        match self {
            TextSize::Compact => "compact",
            TextSize::Standard => "standard",
            TextSize::Large => "large",
        }
    }
    pub fn next(self) -> Self {
        match self {
            TextSize::Compact => TextSize::Standard,
            TextSize::Standard => TextSize::Large,
            TextSize::Large => TextSize::Compact,
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        [TextSize::Compact, TextSize::Standard, TextSize::Large].into_iter().find(|s| s.name() == name)
    }
}

/// The type roles at one text size.
#[derive(Clone, Copy)]
pub struct Type {
    /// The smallest text: lowercase mono captions and hints.
    pub caption: TextStyle,
    /// Mono values.
    pub label: TextStyle,
    /// Rows and control labels.
    pub body: TextStyle,
    /// Names of things.
    pub name: TextStyle,
    /// Screen titles.
    pub title: TextStyle,
    /// Multiplies display (glanceable value) sizes.
    pub display_scale: f32,
}

impl Type {
    pub fn of(size: TextSize) -> Type {
        let (caption, label, body, name, title, display_scale) = match size {
            TextSize::Compact => (10.0, 11.0, 12.0, 13.0, 15.0, 0.9),
            // The default: nothing under 11 px (about 1.5 mm on this panel),
            // and no larger than the screen can carry without cutting words.
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

/// Where a line's baseline goes to sit centred on `cy`.
fn baseline_for(style: &TextStyle, cy: f32) -> f32 {
    (cy + style.size * 0.36).round()
}

pub struct Gfx {
    pub canvas: Canvas,
    pub text: Text,
    pub icons: Icons,
    pub glass: Glass,
    pub tokens: Tokens,
    /// The accent the palette was grown from.
    pub accent: bezel::Accent,
    pub ty: Type,
    pub text_size: TextSize,
    /// Horizontal offset of what is being drawn: screens slide during a page turn.
    pub ox: f32,
    /// Reduced transparency and motion.
    pub calm: bool,
    /// Page dots a screen wants on the glass pager: (pages, current, x offset).
    pub pager: Option<(usize, usize, f32)>,
    /// The width a screen title may take before it meets the island.
    pub title_room: f32,
}

impl Gfx {
    pub fn new(tokens: Tokens) -> Self {
        Gfx {
            canvas: Canvas::new(W as i32, H as i32),
            text: Text::new(),
            icons: Icons::new(),
            glass: Glass::new(),
            tokens,
            accent: bezel::Accent::Orange,
            ty: Type::of(TextSize::Standard),
            text_size: TextSize::Standard,
            ox: 0.0,
            calm: false,
            pager: None,
            title_room: 96.0,
        }
    }

    pub fn set_text_size(&mut self, size: TextSize) {
        self.text_size = size;
        self.ty = Type::of(size);
    }

    // ---- layout, measured from the type ----

    /// The glass island's height.
    pub fn island_h(&self) -> f32 {
        self.ty.caption.size + 8.0
    }
    /// Where a screen's content starts, below the island.
    pub fn top(&self) -> f32 {
        self.island_h() + 9.0
    }
    /// The height of a one-line list row.
    pub fn row_h(&self) -> f32 {
        self.ty.body.size + 10.0
    }
    /// The space a hint line takes at the bottom of a page.
    pub fn hint_h(&self) -> f32 {
        self.ty.caption.size + 4.0
    }

    // ---- drawing ----

    pub fn surface(&mut self, r: Rect, radius: f32, color: Color) {
        self.canvas.rrect(r.offset(self.ox, 0.0), radius, color, 1.0);
    }

    pub fn text(&mut self, x: f32, baseline: f32, s: &str, style: TextStyle, color: Color) -> f32 {
        self.text.draw(&mut self.canvas, x + self.ox, baseline, s, &style, color, 1.0)
    }

    /// Text placed by its left, centre or right, centred vertically on `cy`.
    pub fn text_at(&mut self, anchor: f32, cy: f32, align: Align, s: &str, style: TextStyle, color: Color) -> f32 {
        self.text.draw_aligned(&mut self.canvas, anchor + self.ox, baseline_for(&style, cy), align, s, &style, color, 1.0)
    }

    /// Left-aligned text centred on `cy`, cut with an ellipsis to fit `max_width`.
    pub fn text_fit(&mut self, x: f32, cy: f32, s: &str, style: TextStyle, color: Color, max_width: f32) -> f32 {
        let fitted = self.text.fit(s, &style, max_width.max(0.0));
        self.text_at(x, cy, Align::Left, &fitted, style, color)
    }

    pub fn measure(&mut self, s: &str, style: TextStyle) -> f32 {
        self.text.measure(s, &style)
    }

    pub fn icon(&mut self, name: &str, filled: bool, cx: f32, cy: f32, size: f32, color: Color) {
        self.icons.draw(&mut self.canvas, name, filled, cx + self.ox, cy, size, color, 1.0);
    }

    pub fn dot(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        self.canvas.circle(cx + self.ox, cy, radius, color, 1.0);
    }

    /// The focus ring: signal, just outside the focused surface. It lands at
    /// once: Bezel does not animate changes that come from keys.
    pub fn focus_ring(&mut self, r: Rect, radius: f32) {
        let signal = self.tokens.signal;
        self.canvas.stroke_rrect(r.inset(-2.5).offset(self.ox, 0.0), radius + 2.5, 1.5, signal, 1.0);
    }

    /// A meter: the fill takes the track's outer corners through a clip.
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

    /// A tile holding one glanceable value: a quiet label above, the value in
    /// the display voice, and its unit.
    pub fn stat(&mut self, r: Rect, label: &str, value: Option<String>, unit: &str, size: f32) {
        let t = self.tokens;
        let ty = self.ty;
        self.surface(r, TILE_RADIUS, t.surface1);
        self.text_at(r.x + 9.0, r.y + 5.0 + ty.caption.size * 0.5, Align::Left, label, ty.caption, t.dim);
        let display = ty.display(size);
        let baseline = r.bottom() - 8.0;
        match value {
            Some(v) => {
                let w = self.text(r.x + 8.0, baseline, &v, display, t.ink);
                self.text(r.x + 10.0 + w, baseline, unit, ty.caption, t.dim);
            }
            // Absent, never 0.
            None => {
                self.text(r.x + 8.0, baseline, "–", display, t.faint);
            }
        }
    }

    /// One row of a list: a name on the left, a mono value on the right.
    pub fn row(&mut self, r: Rect, name: &str, value: &str, value_color: Color, focused: bool) {
        let t = self.tokens;
        let ty = self.ty;
        let radius = (r.h * 0.5).min(11.0);
        self.surface(r, radius, if focused { t.surface2 } else { t.surface1 });
        let value_w = self.measure(value, ty.label);
        self.text_fit(r.x + 10.0, r.cy(), name, ty.body, t.ink, r.w - 30.0 - value_w);
        self.text_at(r.right() - 10.0, r.cy(), Align::Right, value, ty.label, value_color);
        if focused {
            self.focus_ring(r, radius);
        }
    }

    /// The hint line at the bottom of a page: lowercase mono, quiet.
    pub fn hint(&mut self, s: &str) {
        let (style, color) = (self.ty.caption, self.tokens.dim);
        self.text_at(W * 0.5, H - 2.0 - style.size * 0.5, Align::Center, s, style, color);
    }

    /// What a page says when it has nothing to show.
    pub fn empty(&mut self, icon: &str, line: &str) {
        let t = self.tokens;
        let cy = (self.top() + H) * 0.5;
        self.icon(icon, false, W * 0.5, cy - 12.0, 24.0, t.faint);
        let style = self.ty.caption;
        self.text_at(W * 0.5, cy + 12.0, Align::Center, line, style, t.dim);
    }

    /// Run the glass pass over everything drawn so far.
    pub fn glass(&mut self, shapes: &[GlassShape]) {
        self.glass.render(&mut self.canvas, shapes, &self.tokens, bezel::LIGHT_REST, self.calm);
    }
}

/// A focus index over a scrolling list of fixed-height rows.
#[derive(Default)]
pub struct ListState {
    pub focus: usize,
    first: usize,
    visible: usize,
}

impl ListState {
    pub fn up(&mut self) {
        self.focus = self.focus.saturating_sub(1);
    }
    pub fn down(&mut self, len: usize) {
        if self.focus + 1 < len {
            self.focus += 1;
        }
    }
    /// Move by a page: as many rows as were last on screen.
    pub fn page(&mut self, len: usize, direction: isize) {
        let step = self.visible.max(1) as isize * direction;
        self.focus = (self.focus as isize + step).clamp(0, len.saturating_sub(1) as isize) as usize;
    }
    /// The rows to draw: (index, y). Keeps the focus on screen.
    pub fn layout(&mut self, len: usize, top: f32, row_h: f32, gap: f32, bottom: f32) -> Vec<(usize, f32)> {
        self.focus = self.focus.min(len.saturating_sub(1));
        self.visible = (((bottom - top) + gap) / (row_h + gap)).floor().max(1.0) as usize;
        if self.focus < self.first {
            self.first = self.focus;
        } else if self.focus >= self.first + self.visible {
            self.first = self.focus + 1 - self.visible;
        }
        self.first = self.first.min(len.saturating_sub(self.visible));
        (self.first..len.min(self.first + self.visible)).map(|i| (i, top + (i - self.first) as f32 * (row_h + gap))).collect()
    }
    /// A thin position marker when the list is longer than the page.
    pub fn scrollbar(&self, g: &mut Gfx, len: usize, top: f32, bottom: f32) {
        if len <= self.visible {
            return;
        }
        let track = bottom - top;
        let thumb = (track * self.visible as f32 / len as f32).max(10.0);
        let y = top + (track - thumb) * self.first as f32 / (len - self.visible) as f32;
        let faint = g.tokens.faint;
        g.surface(Rect::new(W - 5.0, y, 2.5, thumb), 1.25, faint);
    }
}
