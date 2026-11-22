//! The window's controls, drawn as the flasher draws (bezel's canvas and
//! text, Bezel's dark tokens): text, cards, pill buttons with a state
//! layer, text fields, segmented choices, progress, icons. Immediate mode:
//! each control is drawn and asked about the mouse and keys in one call.

use bezel::{Align, Canvas, Color, Icons, Rect, Text, TextStyle, Tokens};

pub const W: usize = 960;
pub const H: usize = 620;
pub const CRIMSON: Color = Color::hex(0xe94560);
pub const GOOD: Color = Color::hex(0x5ccf8e);

/// A key the controls take (letters come as typed characters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum K {
    Tab,
    ShiftTab,
    Enter,
    Backspace,
    Delete,
    Escape,
    Paste,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    Quiet,
    Danger,
}

/// One frame's input and the means to draw.
pub struct Ui<'a> {
    pub canvas: &'a mut Canvas,
    pub text: &'a mut Text,
    pub icons: &'a mut Icons,
    pub t: Tokens,
    pub mouse: (f32, f32),
    /// The left button went down this frame.
    pub clicked: bool,
    pub wheel: f32,
    pub typed: Vec<char>,
    pub keys: Vec<K>,
    /// The text field typed into, by its id.
    pub focus: &'a mut Option<String>,
    /// Fields drawn this frame, in order (Tab goes through them).
    pub fields: Vec<String>,
    /// A control the mouse is over wants the hand.
    pub pointing: bool,
    /// Seconds since the window opened (the caret blinks by it).
    pub time: f32,
    /// What the clipboard held, if Ctrl+V was pressed.
    pub pasted: Option<String>,
}

impl Ui<'_> {
    /// Text centred on `cy`.
    pub fn put(&mut self, x: f32, cy: f32, s: &str, style: TextStyle, color: Color, align: Align) -> f32 {
        self.text.draw_aligned(self.canvas, x, (cy + style.size * 0.36).round(), align, s, &style, color, 1.0)
    }

    pub fn put_alpha(&mut self, x: f32, cy: f32, s: &str, style: TextStyle, color: Color, align: Align, alpha: f32) -> f32 {
        self.text.draw_aligned(self.canvas, x, (cy + style.size * 0.36).round(), align, s, &style, color, alpha)
    }

    /// `s` cut with an ellipsis to `width`, then drawn.
    pub fn put_fit(&mut self, x: f32, cy: f32, s: &str, style: TextStyle, color: Color, width: f32) -> f32 {
        let s = self.text.fit(s, &style, width);
        self.put(x, cy, &s, style, color, Align::Left)
    }

    /// Lines wrapped to `width` from `top`; where the last one ended.
    pub fn para(&mut self, x: f32, top: f32, width: f32, s: &str, style: TextStyle, color: Color, line: f32) -> f32 {
        let lines = self.text.wrap(s, &style, width, true);
        let mut y = top + line * 0.5;
        for l in &lines {
            self.put(x, y, l, style, color, Align::Left);
            y += line;
        }
        top + line * lines.len() as f32
    }

    pub fn measure(&mut self, s: &str, style: TextStyle) -> f32 {
        self.text.measure(s, &style)
    }

    pub fn over(&self, r: Rect) -> bool {
        let (x, y) = self.mouse;
        x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
    }

    /// Clicked inside `r` this frame.
    pub fn hit(&mut self, r: Rect) -> bool {
        let over = self.over(r);
        self.pointing |= over;
        over && self.clicked
    }

    pub fn icon(&mut self, name: &str, cx: f32, cy: f32, size: f32, color: Color) {
        self.icons.draw(self.canvas, name, false, cx, cy, size, color, 1.0);
    }

    /// A raised surface.
    pub fn card(&mut self, r: Rect) {
        self.canvas.rrect(r.offset(0.0, 2.0).inset(-1.0), 18.0, Color::BLACK, 0.22);
        self.canvas.rrect(r, 16.0, self.t.surface1, 1.0);
    }

    /// A pill button; true when clicked (and it is enabled).
    pub fn button(&mut self, r: Rect, label: &str, kind: Kind, enabled: bool) -> bool {
        let over = enabled && self.over(r);
        let t = self.t;
        let (fill, ink) = match kind {
            Kind::Primary => (CRIMSON, Color::WHITE),
            Kind::Secondary => (t.surface3, t.ink),
            Kind::Quiet => (t.surface1, t.ink),
            Kind::Danger => (t.surface3, CRIMSON.mix(Color::WHITE, 0.25)),
        };
        let radius = r.h * 0.5;
        if enabled {
            self.canvas.rrect(r, radius, fill, 1.0);
            if over {
                // The state layer: the ink colour over the fill.
                self.canvas.rrect(r, radius, ink, 0.08);
            }
        } else {
            self.canvas.rrect(r, radius, t.surface2, 1.0);
        }
        let style = TextStyle::sans(14.0, 600.0);
        let label = self.text.fit(label, &style, r.w - 24.0);
        self.put(r.cx(), r.cy(), &label, style, if enabled { ink } else { t.faint }, Align::Center);
        self.pointing |= over;
        over && self.clicked
    }

    /// The width a button's label wants.
    pub fn button_w(&mut self, label: &str) -> f32 {
        self.measure(label, TextStyle::sans(14.0, 600.0)) + 36.0
    }

    /// A line of buttons from `x`, left to right; which was clicked.
    pub fn buttons(&mut self, x: f32, y: f32, items: &[(&str, Kind, bool)]) -> Option<usize> {
        let mut at = x;
        let mut clicked = None;
        for (i, (label, kind, enabled)) in items.iter().enumerate() {
            let w = self.button_w(label).max(if *kind == Kind::Primary { 112.0 } else { 84.0 });
            if self.button(Rect::new(at, y, w, 36.0), label, *kind, *enabled) {
                clicked = Some(i);
            }
            at += w + 10.0;
        }
        clicked
    }

    /// A one-line text field. Clicking it takes the keys; true on Enter.
    pub fn field(&mut self, id: &str, r: Rect, placeholder: &str, value: &mut String, secret: bool) -> bool {
        self.fields.push(id.to_owned());
        if self.hit(r) {
            *self.focus = Some(id.to_owned());
        }
        let focused = self.focus.as_deref() == Some(id);
        let mut entered = false;
        if focused {
            for c in std::mem::take(&mut self.typed) {
                if !c.is_control() && value.chars().count() < 512 {
                    value.push(c);
                }
            }
            if let Some(p) = self.pasted.take() {
                value.push_str(p.trim_matches(['\r', '\n']).lines().next().unwrap_or_default());
            }
            for k in self.keys.clone() {
                match k {
                    K::Backspace => {
                        value.pop();
                    }
                    K::Enter => entered = true,
                    _ => {}
                }
            }
        }
        let t = self.t;
        self.canvas.rrect(r, 10.0, t.surface2, 1.0);
        if focused {
            self.canvas.stroke_rrect(r.inset(-1.5), 11.5, 1.5, CRIMSON, 1.0);
        } else if self.over(r) {
            self.canvas.stroke_rrect(r, 10.0, 1.0, t.faint, 0.6);
        }
        let style = TextStyle::body(14.0);
        let shown: String = if secret { "•".repeat(value.chars().count()) } else { value.clone() };
        let width = r.w - 24.0;
        // The end of what was typed stays in view.
        let mut visible = shown.clone();
        while !visible.is_empty() && self.measure(&visible, style) > width {
            visible.remove(0);
        }
        if shown.is_empty() {
            let p = self.text.fit(placeholder, &style, width);
            self.put(r.x + 12.0, r.cy(), &p, style, t.faint, Align::Left);
        } else {
            self.put(r.x + 12.0, r.cy(), &visible, style, t.ink, Align::Left);
        }
        if focused && (self.time * 1.6).fract() < 0.6 {
            let x = r.x + 12.0 + if shown.is_empty() { 0.0 } else { self.measure(&visible, style) } + 1.0;
            self.canvas.fill_rect(Rect::new(x, r.cy() - 9.0, 1.5, 18.0), CRIMSON, 1.0);
        }
        entered
    }

    /// Choices side by side; the one clicked.
    pub fn segmented(&mut self, r: Rect, options: &[&str], selected: usize) -> Option<usize> {
        let t = self.t;
        self.canvas.rrect(r, r.h * 0.5, t.surface2, 1.0);
        let w = r.w / options.len() as f32;
        let mut clicked = None;
        for (i, o) in options.iter().enumerate() {
            let cell = Rect::new(r.x + i as f32 * w, r.y, w, r.h);
            if i == selected {
                self.canvas.rrect(cell.inset(3.0), (r.h - 6.0) * 0.5, t.surface3, 1.0);
            } else if self.over(cell) {
                self.canvas.rrect(cell.inset(3.0), (r.h - 6.0) * 0.5, t.ink, 0.05);
            }
            let style = TextStyle::sans(13.0, if i == selected { 650.0 } else { 500.0 });
            self.put(cell.cx(), cell.cy(), o, style, if i == selected { t.ink } else { t.dim }, Align::Center);
            if self.hit(cell) && i != selected {
                clicked = Some(i);
            }
        }
        clicked
    }

    /// A switch; true when clicked.
    pub fn toggle(&mut self, x: f32, cy: f32, on: bool) -> bool {
        let track = Rect::new(x, cy - 11.0, 40.0, 22.0);
        let t = self.t;
        self.canvas.rrect(track, 11.0, if on { CRIMSON } else { t.surface3 }, 1.0);
        let knob_x = if on { track.right() - 11.0 } else { track.x + 11.0 };
        self.canvas.circle(knob_x, cy, 8.0, Color::WHITE, 1.0);
        self.hit(track.inset(-4.0))
    }

    pub fn progress(&mut self, r: Rect, fraction: f32) {
        let t = self.t;
        self.canvas.rrect(r, r.h * 0.5, t.surface3, 1.0);
        let w = (r.w * fraction.clamp(0.0, 1.0)).max(r.h);
        self.canvas.rrect_clipped(Rect::new(r.x, r.y, w, r.h), r.h * 0.5, r, r.h * 0.5, CRIMSON, 1.0);
    }

    /// A small round mark of how something stands.
    pub fn dot(&mut self, cx: f32, cy: f32, color: Color) {
        self.canvas.circle(cx, cy, 4.0, color, 1.0);
    }
}

/// The ground: Bezel's dark, with the faintest crimson from the top left.
pub fn ground(canvas: &mut Canvas, t: &Tokens) {
    canvas.clear(t.ground);
    canvas.glow(0.0, 0.0, 520.0, CRIMSON, 0.07);
}

/// The bolt of the Catalyst mark.
pub fn bolt(canvas: &mut Canvas, cx: f32, cy: f32, size: f32) {
    let s = size / 40.0;
    let pts = [(4.0, -20.0), (-11.0, 3.0), (-1.0, 3.0), (-4.0, 20.0), (11.0, -3.0), (1.0, -3.0)].map(|(x, y)| (cx + x * s, cy + y * s));
    canvas.polygon(&pts, CRIMSON, 1.0);
}

/// Bytes, as a person reads them.
pub fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} bytes"),
    }
}
