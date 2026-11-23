//! Mech's pages: the mechanisms to work out and those kept, and one
//! mechanism's page, its numbers on the right and what they give on the
//! left (two figures and a graph), worked out again as each is typed.

use bezel::{Align, Rect};
use detent::{role, Animated};

use super::model::{self, StageKind};
use super::{Calc, FieldKind, Mech, Plot, Solved};
use crate::gfx::{Gfx, ListState, GAP, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field as Line, Typed};

/// The card's width on the left of a mechanism's page.
const CARD_W: f32 = 140.0;
/// The graph's line, drawn as this many points (so a change can glide).
const POINTS: usize = 64;

// ------------------------------------------------------------------ list ---

/// The calculators, and the mechanisms kept.
pub struct MechApp {
    list: ListState,
    saved: Vec<(std::path::PathBuf, Mech)>,
    /// Backspace pressed once on a kept mechanism: once more forgets it.
    forgetting: Option<usize>,
}

impl MechApp {
    pub fn new() -> Self {
        MechApp { list: ListState::default(), saved: super::saved(), forgetting: None }
    }

    /// Rows: the calculators, a heading, then what is kept.
    fn len(&self) -> usize {
        Calc::ALL.len() + if self.saved.is_empty() { 0 } else { 1 + self.saved.len() }
    }

    fn heading(i: usize) -> bool {
        i == Calc::ALL.len()
    }
}

impl Screen for MechApp {
    fn title(&self) -> &str {
        "Mech"
    }

    fn resume(&mut self) {
        self.saved = super::saved();
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.forgetting.is_some() {
            return vec![keys("forgetting", &[("del", "Forget it"), ("esc", "Keep it")])];
        }
        if self.saved.is_empty() {
            return vec![keys("mechanisms", &[("enter", "Work one out"), ("↑ ↓", "Move")])];
        }
        vec![keys("mechanisms", &[("enter", "Open"), ("↑ ↓", "Move"), ("del twice", "Forget a kept one")])]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let len = self.len();
        let at = self.list.focus;
        // Backspace comes as Esc here (the shell's), so Fn-Backspace forgets.
        let forgetting = self.forgetting;
        if !matches!(key, Key::Backspace | Key::Delete) {
            self.forgetting = None;
        }
        match key {
            Key::Back if forgetting.is_some() => Nav::Stay,
            Key::Back | Key::Leave => Nav::Pop,
            Key::Enter | Key::Right => match at.checked_sub(Calc::ALL.len() + 1) {
                Some(k) => self.saved.get(k).map_or(Nav::Stay, |(_, m)| Nav::Push(Box::new(Form::new(m.clone())))),
                None => Calc::ALL.get(at).map_or(Nav::Stay, |c| Nav::Push(Box::new(Form::new(Mech::new(*c))))),
            },
            Key::Backspace | Key::Delete => {
                if let Some(k) = at.checked_sub(Calc::ALL.len() + 1) {
                    if self.forgetting == Some(k) {
                        if let Some((path, m)) = self.saved.get(k) {
                            let gone = std::fs::remove_file(path).is_ok();
                            *cx.say = Some(if gone { format!("{} forgotten", m.name) } else { "could not forget it".into() });
                        }
                        self.forgetting = None;
                        self.saved = super::saved();
                        self.list.focus = self.list.focus.min(self.len().saturating_sub(1));
                    } else {
                        self.forgetting = Some(k);
                    }
                }
                Nav::Stay
            }
            other => {
                crate::tools::step_over_headings(&mut self.list, other, len, Self::heading);
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let len = self.len();
        for (i, y) in self.list.layout(g, len, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            if let Some(c) = Calc::ALL.get(i) {
                g.row(r, c.name(), c.gives(), t.dim, i == self.list.focus);
            } else if Self::heading(i) {
                crate::tools::heading(g, y, row_h, "kept");
            } else if let Some((_, m)) = self.saved.get(i - Calc::ALL.len() - 1) {
                g.row(r, &m.name, &m.calc.name().to_lowercase(), t.dim, i == self.list.focus);
            }
        }
        self.list.scrollbar(g, len, top, bottom);
        if self.forgetting.is_some() {
            g.hint_any(&["del again: forget it · esc: keep it", "del again: forget it"]);
        } else if self.list.focus > Calc::ALL.len() {
            g.hint_any(&["enter: open · del: forget", "enter: open"]);
        } else {
            g.hint_any(&["enter: work one out · ctrl-s in it keeps it", "enter: work one out"]);
        }
    }
}

// ------------------------------------------------------------- mechanism ---

/// One mechanism: its numbers, and what they give.
pub struct Form {
    mech: Mech,
    solved: Solved,
    list: ListState,
    /// What is being typed into the focused number, if anything.
    typing: Option<String>,
    /// Ctrl-S: the name it is kept under, being typed.
    naming: Option<Line>,
    /// The graph's line as last drawn (panel points), and the glide from it
    /// to the new one after a change.
    drawn: Vec<(f32, f32)>,
    from: Vec<(f32, f32)>,
    glide: Animated,
    calm: bool,
}

impl Form {
    pub fn new(mech: Mech) -> Self {
        let solved = mech.solve();
        Form { mech, solved, list: ListState::default(), typing: None, naming: None, drawn: Vec::new(), from: Vec::new(), glide: Animated::new(1.0, role::settle()).with_rest(0.002), calm: false }
    }

    /// The page as if keys had been pressed: a field in focus, perhaps typing.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn at(mut self, focus: usize, typing: Option<&str>) -> Self {
        self.list.focus = focus;
        self.typing = typing.map(str::to_owned);
        self
    }

    #[cfg(not(target_arch = "aarch64"))]
    pub fn naming(mut self, name: &str) -> Self {
        self.naming = Some(Line::new("keep as", name, 32));
        self
    }

    fn fields(&self) -> &'static [super::Field] {
        self.mech.calc.fields()
    }

    fn len(&self) -> usize {
        self.fields().len() + 1 + self.solved.rows.len()
    }

    fn field(&self) -> Option<super::Field> {
        self.fields().get(self.list.focus).copied()
    }

    /// The numbers changed: work it out again, the graph gliding over.
    fn changed(&mut self) {
        self.solved = self.mech.solve();
        if !self.calm && self.drawn.len() == POINTS {
            self.from = self.drawn.clone();
            self.glide.jump(0.0);
            self.glide.set(1.0);
        }
    }

    /// What is typed goes in, if it fits the field; else it is let go.
    fn commit(&mut self) {
        let Some(text) = self.typing.take() else { return };
        let Some(f) = self.field() else { return };
        match f.kind {
            FieldKind::Stage(k) => {
                let stage = if text.trim().is_empty() {
                    None
                } else {
                    let kind = self.mech.stages[k].map_or(StageKind::Gear, |s| s.kind);
                    match super::parse_teeth(&text) {
                        Some((driving, driven)) => Some(model::Stage { kind, driving, driven }),
                        None => return,
                    }
                };
                // The train keeps one stage at least.
                if stage.is_none() && self.mech.stages.iter().enumerate().all(|(i, s)| i == k || s.is_none()) {
                    return;
                }
                self.mech.stages[k] = stage;
            }
            _ => {
                let Ok(v) = text.replace(',', ".").parse::<f64>() else { return };
                let Some(v) = f.accept(v) else { return };
                self.mech.values[self.list.focus] = v;
            }
        }
        self.changed();
    }

    /// The focused field's value as its row shows it.
    fn shown(&self, i: usize) -> String {
        let f = &self.fields()[i];
        match f.kind {
            FieldKind::Motor => String::new(),
            FieldKind::Stage(k) => self.mech.stages[k].map(|s| format!("{}:{}", s.driving, s.driven)).unwrap_or_else(|| "–".into()),
            _ => f.show(self.mech.values[i]),
        }
    }

    /// A row's name: a stage goes by its kind (`belt 2`).
    fn label(&self, i: usize) -> String {
        let f = &self.fields()[i];
        match f.kind {
            FieldKind::Stage(k) => match self.mech.stages[k] {
                Some(s) => format!("{} {}", s.kind.name(), k + 1),
                None => format!("stage {}", k + 1),
            },
            _ => f.label.to_owned(),
        }
    }

    /// Left and Right: the motor, a stage's kind, a count.
    fn step(&mut self, by: isize) {
        let Some(f) = self.field() else { return };
        match f.kind {
            FieldKind::Motor => {
                let n = model::motors().len() as isize;
                self.mech.motor = (self.mech.motor as isize + by).rem_euclid(n) as usize;
            }
            FieldKind::Stage(k) => {
                let Some(s) = self.mech.stages[k].as_mut() else { return };
                let at = StageKind::ALL.iter().position(|x| *x == s.kind).unwrap_or(0) as isize;
                s.kind = StageKind::ALL[(at + by).rem_euclid(3) as usize];
            }
            FieldKind::Count => {
                let v = &mut self.mech.values[self.list.focus];
                *v = (*v + by as f64).clamp(1.0, 12.0);
            }
            _ => return,
        }
        self.changed();
    }

    fn hint(&self, g: &mut Gfx) {
        if self.naming.is_some() {
            return g.hint_any(&["enter: keep it · esc: not now", "enter: keep it"]);
        }
        match self.field().map(|f| f.kind) {
            _ if self.typing.is_some() => g.hint_any(&["enter: done · esc: as it was · tab: next", "enter: done · esc: as it was", "enter: done"]),
            Some(FieldKind::Motor) => g.hint_any(&["←→: another motor · tab: next · ctrl-s: keep", "←→: another motor · tab: next", "←→: motor"]),
            Some(FieldKind::Stage(_)) => g.hint_any(&["type 12:60 · ←→: gear, belt, chain · tab: next", "type 12:60 · ←→: kind", "type 12:60"]),
            Some(FieldKind::Count) => g.hint_any(&["type it or ←→ · tab: next · ctrl-s: keep", "type it or ←→ · tab: next", "type or ←→"]),
            Some(_) => g.hint_any(&["type a number · tab: next · ctrl-s: keep", "type a number · tab: next", "type a number"]),
            None => g.hint_any(&["↑↓: results · tab: the numbers · ctrl-s: keep", "tab: the numbers · ctrl-s: keep", "tab: the numbers"]),
        }
    }
}

impl Screen for Form {
    fn title(&self) -> &str {
        if self.mech.name.is_empty() {
            self.mech.calc.short()
        } else {
            &self.mech.name
        }
    }

    fn app(&self) -> &str {
        "Mech"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.glide.tick(dt)
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.naming.is_some() {
            return vec![keys("keeping it", &[("enter", "Keep it"), ("esc", "Not now")])];
        }
        let here = match self.field().map(|f| f.kind) {
            _ if self.typing.is_some() => keys("typing", &[("enter", "Done"), ("tab", "Done, next number"), ("esc", "As it was"), ("bksp", "Take a figure off")]),
            Some(FieldKind::Motor) => keys("the motor", &[("← →", "Another motor"), ("tab", "Next number")]),
            Some(FieldKind::Stage(_)) => keys("a stage", &[("type 12:60", "Its teeth"), ("← →", "Gear, belt or chain"), ("bksp", "Change it"), ("tab", "Next number")]),
            Some(FieldKind::Count) => keys("a count", &[("type · ← →", "Change it"), ("tab", "Next number")]),
            Some(_) => keys("a number", &[("type 2.5", "A new value"), ("bksp", "Change it"), ("tab", "Next number")]),
            None => keys("results", &[("tab", "Back to the numbers")]),
        };
        vec![here, keys("mechanism", &[("↑ ↓", "Move"), ("ctrl s", "Keep it")])]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(line) = &mut self.naming {
            match line.key(key, |c| c.is_alphanumeric() || " -_'".contains(c)) {
                Typed::More => {}
                Typed::Cancelled => self.naming = None,
                Typed::Done(name) if name.is_empty() => {}
                Typed::Done(name) => {
                    self.naming = None;
                    self.mech.name = name;
                    *cx.say = Some(match super::save(&self.mech) {
                        Ok(_) => format!("{} kept", self.mech.name),
                        Err(e) => format!("could not keep it: {e}"),
                    });
                }
            }
            return Nav::Stay;
        }
        let nf = self.fields().len();
        let len = self.len();
        let kind = self.field().map(|f| f.kind);
        match key {
            Key::Back if self.typing.is_some() => self.typing = None,
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Ctrl('s') => {
                self.commit();
                let name = if self.mech.name.is_empty() { self.mech.calc.name().to_owned() } else { self.mech.name.clone() };
                self.naming = Some(Line::new("keep as", &name, 32));
            }
            Key::Tab | Key::Enter => {
                self.commit();
                // Tab: the next number (round to the first); Enter stays.
                if key == Key::Tab {
                    self.list.focus = if self.list.focus + 1 >= nf { 0 } else { self.list.focus + 1 };
                }
            }
            Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Home | Key::End => {
                self.commit();
                let heading = nf;
                crate::tools::step_over_headings(&mut self.list, key, len, |i| i == heading);
            }
            Key::Left | Key::Right if self.typing.is_none() => self.step(if key == Key::Left { -1 } else { 1 }),
            Key::Backspace => match &mut self.typing {
                Some(text) => {
                    text.pop();
                }
                // Backspace on a value takes its last figure off, to type on.
                None if kind.is_some_and(|k| k != FieldKind::Motor) => {
                    let mut text = match kind {
                        Some(FieldKind::Stage(k)) => self.mech.stages[k].map(|s| format!("{}:{}", s.driving, s.driven)).unwrap_or_default(),
                        _ => super::trim(self.mech.values[self.list.focus], 6),
                    };
                    text.pop();
                    self.typing = Some(text);
                }
                None => {}
            },
            Key::Char(c) => {
                let allowed = match kind {
                    Some(FieldKind::Stage(_)) => c.is_ascii_digit() || c == ':',
                    Some(FieldKind::Signed(_)) => c.is_ascii_digit() || c == '.' || c == ',' || c == '-',
                    Some(FieldKind::Motor) | None => false,
                    Some(_) => c.is_ascii_digit() || c == '.' || c == ',',
                };
                if allowed {
                    let text = self.typing.get_or_insert_with(String::new);
                    if text.len() < 9 {
                        text.push(c);
                    }
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        self.calm = g.calm;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let nf = self.fields().len();
        let len = self.len();
        let x = PAD + CARD_W + GAP;
        let w = W - PAD - 5.0 - x;
        for (i, y) in self.list.layout(g, len, top, row_h, 3.0, bottom) {
            let r = Rect::new(x, y, w, row_h);
            let focused = i == self.list.focus;
            if i < nf {
                let f = self.fields()[i];
                if focused && self.typing.is_some() {
                    let typed = self.typing.clone().unwrap_or_default();
                    let label = self.label(i);
                    g.row(r, &label, "", t.ink, true);
                    let right = r.right() - 10.0;
                    g.surface(Rect::new(right - 1.5, r.cy() - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
                    g.text_at(right - 4.0, r.cy(), Align::Right, &typed, ty.label, t.ink);
                } else if f.kind == FieldKind::Motor {
                    let name = self.mech.motor().name.clone();
                    g.row(r, &name, "‹ ›", t.dim, focused);
                } else {
                    let shown = self.shown(i);
                    let label = self.label(i);
                    g.row(r, &label, &shown, t.ink, focused);
                }
            } else if i == nf {
                g.text_at(x + 4.0, y + row_h * 0.62, Align::Left, "results", ty.caption, t.dim);
            } else if let Some((name, value)) = self.solved.rows.get(i - nf - 1) {
                let (name, value) = (name.clone(), value.clone());
                g.row(r, &name, &value, t.dim, focused);
            }
        }
        self.list.scrollbar(g, len, top, bottom);
        // The card: two figures, then the graph.
        let card = Rect::new(PAD, top, CARD_W, bottom - top);
        let radius = g.radius(12.0);
        g.surface(card, radius, t.surface1);
        let inner = card.inset(8.0);
        let cap = ty.caption;
        // Each figure on a line of its own: what it is on the left, its
        // value (and unit) on the right.
        let big = ty.display(15.0);
        let line_h = big.size + 4.0;
        // Under the name being typed, only the graph.
        let heads = if self.naming.is_some() { &[][..] } else { &self.solved.heads[..] };
        for (k, (what, value, unit)) in heads.iter().enumerate() {
            let cy = inner.y + line_h * (k as f32 + 0.5);
            let unit_w = if unit.is_empty() { 0.0 } else { g.measure(unit, cap) + 3.0 };
            let value_w = g.text_at(inner.right() - unit_w, cy, Align::Right, value, big, t.ink);
            if !unit.is_empty() {
                g.text_at(inner.right(), cy + big.size * 0.15, Align::Right, unit, cap, t.dim);
            }
            g.text_fit(inner.x, cy, what, cap, t.dim, inner.w - unit_w - value_w - 6.0);
        }
        let heads_h = 2.0 * line_h + 4.0;
        let area = Rect::new(inner.x, inner.y + heads_h, inner.w, inner.h - heads_h);
        self.drawn = plot(g, area, &self.solved.plot, &self.from, self.glide.value());
        if let Some(line) = &self.naming {
            line.draw_at(g, true);
        }
        self.hint(g);
    }
}

/// The smallest of 1, 2, 2.5 and 5 times a power of ten at least `x`.
pub fn nice(x: f64) -> f64 {
    if !(x > 0.0) || !x.is_finite() {
        return 1.0;
    }
    let p = 10f64.powf(x.log10().floor());
    [1.0, 2.0, 2.5, 5.0, 10.0].into_iter().map(|m| m * p).find(|v| *v >= x * 0.999_999).unwrap_or(10.0 * p)
}

/// A number on an axis: no more figures than it needs.
fn tick_label(v: f64) -> String {
    if v >= 1000.0 {
        super::trim(v, 0)
    } else if v >= 10.0 {
        super::trim(v, 1)
    } else {
        super::trim(v, 2)
    }
}

/// `pts` as `n` points evenly along their length.
fn resample(pts: &[(f32, f32)], n: usize) -> Vec<(f32, f32)> {
    if pts.len() < 2 {
        return vec![pts.first().copied().unwrap_or_default(); n];
    }
    let seg: Vec<f32> = pts.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).collect();
    let total: f32 = seg.iter().sum::<f32>().max(1e-6);
    let mut out = Vec::with_capacity(n);
    let (mut i, mut walked) = (0usize, 0.0f32);
    for k in 0..n {
        let want = total * k as f32 / (n - 1) as f32;
        while i + 1 < seg.len() && walked + seg[i] < want {
            walked += seg[i];
            i += 1;
        }
        let f = if seg[i] > 0.0 { ((want - walked) / seg[i]).clamp(0.0, 1.0) } else { 0.0 };
        let (a, b) = (pts[i], pts[i + 1]);
        out.push((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f));
    }
    out
}

/// The graph in `r`: one axis each way, the line in the signal colour (a
/// faint one where the current limit cuts it), where the mechanism works
/// marked, and a level across. Returns the line as drawn, in panel points.
fn plot(g: &mut Gfx, r: Rect, p: &Plot, from: &[(f32, f32)], glide: f32) -> Vec<(f32, f32)> {
    let t = g.tokens;
    let cap = g.ty.caption;
    let all = p.line.iter().chain(&p.ghost).chain(p.mark.iter());
    let (mut x_lo, mut x_hi, mut y_hi) = (f64::MAX, f64::MIN, 0.0f64);
    for (x, y) in all {
        x_lo = x_lo.min(*x);
        x_hi = x_hi.max(*x);
        y_hi = y_hi.max(*y);
    }
    // A level far above the line would flatten it: shown only near it.
    let level = p.level.filter(|(l, _)| *l <= y_hi * 2.0);
    if let Some((l, _)) = level {
        y_hi = y_hi.max(l);
    }
    if p.line.is_empty() || x_hi <= x_lo {
        g.text_at(r.cx(), r.cy(), Align::Center, "nothing to draw", cap, t.dim);
        return Vec::new();
    }
    // An axis from zero, unless it runs negative (an arm's angles).
    let (x_lo, x_hi) = if p.exact_x || x_lo < 0.0 { (x_lo, x_hi) } else { (0.0, nice(x_hi)) };
    let y_hi = nice(y_hi);
    // Words above and below; the plot between.
    let area = Rect::new(r.x + 1.0, r.y + cap.size + 4.0, r.w - 2.0, r.h - 2.0 * cap.size - 8.0);
    let sx = |x: f64| area.x + ((x - x_lo) / (x_hi - x_lo)) as f32 * area.w;
    let sy = |y: f64| area.bottom() - (y / y_hi).clamp(-0.05, 1.05) as f32 * area.h;
    g.text_at(r.x, r.y + cap.size * 0.5, Align::Left, &format!("{} {}", tick_label(y_hi), p.y_unit), cap, t.dim);
    let ox = g.ox;
    // The axes: recessive hairlines.
    g.canvas.fill_rect(Rect::new(area.x + ox, area.bottom(), area.w, 1.0), t.faint, 0.9);
    g.canvas.fill_rect(Rect::new(area.x + ox, area.y, 1.0, area.h), t.faint, 0.9);
    let under = area.bottom() + 3.0 + cap.size * 0.5;
    let lo = if x_lo == 0.0 { "0".to_owned() } else if x_lo < 0.0 { format!("-{}", tick_label(-x_lo)) } else { tick_label(x_lo) };
    let lo = if p.x_unit == "°" { format!("{lo}°") } else { lo };
    g.text_at(area.x, under, Align::Left, &lo, cap, t.dim);
    let hi = if p.x_unit == "°" { format!("{}°", tick_label(x_hi)) } else { format!("{} {}", tick_label(x_hi), p.x_unit) };
    g.text_at(area.right(), under, Align::Right, &hi, cap, t.dim);
    if let Some((level, word)) = level {
        if level > 0.0 && level <= y_hi {
            let y = sy(level).round();
            let mut x = area.x + 2.0;
            while x < area.right() - 1.0 {
                g.canvas.fill_rect(Rect::new(x + ox, y, 3.0f32.min(area.right() - x), 1.0), t.dim, 0.8);
                x += 6.0;
            }
            let wy = if y - area.y > cap.size + 2.0 { y - 2.0 - cap.size * 0.5 } else { y + 2.0 + cap.size * 0.5 };
            g.text_at(area.right(), wy, Align::Right, word, cap, t.dim);
        }
    }
    g.canvas.set_clip(Rect::new(area.x + ox - 2.0, area.y - 3.0, area.w + 4.0, area.h + 5.0));
    if !p.ghost.is_empty() {
        let pts: Vec<(f32, f32)> = p.ghost.iter().map(|(x, y)| (sx(*x) + ox, sy(*y))).collect();
        g.canvas.polyline(&pts, 1.0, t.faint, 1.0);
    }
    let target = resample(&p.line.iter().map(|(x, y)| (sx(*x), sy(*y))).collect::<Vec<_>>(), POINTS);
    let line: Vec<(f32, f32)> = if from.len() == POINTS && glide < 1.0 {
        target.iter().zip(from).map(|(b, a)| (a.0 + (b.0 - a.0) * glide, a.1 + (b.1 - a.1) * glide)).collect()
    } else {
        target
    };
    let shifted: Vec<(f32, f32)> = line.iter().map(|(x, y)| (x + ox, *y)).collect();
    g.canvas.polyline(&shifted, 2.0, t.signal, 1.0);
    g.canvas.clear_clip();
    if let Some((x, y)) = p.mark {
        if glide >= 1.0 || from.len() != POINTS {
            let (cx, cy) = (sx(x) + ox, sy(y));
            g.canvas.circle(cx, cy, 6.0, t.surface1, 1.0);
            g.canvas.circle(cx, cy, 4.0, t.ink, 1.0);
        }
    }
    line
}
