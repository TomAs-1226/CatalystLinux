//! The lyrics page: the song's words over the screen below the island.
//!
//! - **Synced**, the line being sung is large, in ink; the lines round it
//!   are smaller and dimmed (those sung fainter than those to come), fading
//!   out towards the screen's edges. The view rides a Detent spring (Bezel's
//!   smooth role) from line to line, measured in lines, so a seek glides
//!   through the words between rather than jumping, and the line growing
//!   and the one shrinking cross over as it moves. A pause in the singing
//!   shows three dots that fill as it passes.
//! - **Plain** words are scrolled by hand (↑↓), on the same spring.
//! - **None**, an instrumental, still looking, or offline with none kept:
//!   said quietly in the middle.
//!
//! Wrapping is done once per song and text size, and only the lines on
//! screen are drawn: two of them at both sizes while they cross over.

use std::sync::Arc;

use bezel::{Color, TextStyle};
use detent::{role, Animated};

use super::lyrics::{self, Follow, Line, Lyrics, State};
use super::player::Now;
use crate::gfx::{Gfx, H, PAD, W};

/// Between one line and the next.
const GAP: f32 = 7.0;
/// Where the line being sung sits, down the space below the island.
const ANCHOR: f32 = 0.4;
/// The edges the words fade out over.
const EDGE: f32 = 22.0;
/// The words' left edge and width.
const X: f32 = PAD + 6.0;
const WIDTH: f32 = W - 2.0 * X;

pub struct LyricsView {
    /// The song shown, and its words once they are here.
    id: String,
    words: Option<Arc<Lyrics>>,
    state: Option<State>,
    follow: Follow,
    current: Option<usize>,
    /// Where the view is, in lines (synced), springing to the line sung.
    at: Animated,
    /// Plain words: how far scrolled, in pixels.
    scroll: Animated,
    /// The words coming in (a new song, or the words arriving).
    shown: Animated,
    layout: Option<Layout>,
    stamp: u64,
    /// A row of plain words, as last drawn (what a press scrolls by).
    step: f32,
    /// How far through a pause its dots are (a third each).

    dots: u32,
}

/// The words wrapped, at both sizes (synced) or one (plain).
struct Layout {
    key: (usize, u32, u32),
    small: Vec<Vec<String>>,
    large: Vec<Vec<String>>,
}

/// The two voices: the line being sung, and the rest.
fn voices(g: &Gfx) -> (TextStyle, TextStyle) {
    (g.ty.display(20.0), g.ty.body)
}

fn row_h(style: TextStyle) -> f32 {
    (style.size * 1.28).round()
}

/// A line's height in a voice: its rows, or a pause's dots.
fn height(rows: &[String], style: TextStyle) -> f32 {
    if rows.is_empty() {
        (style.size * 0.9).round()
    } else {
        rows.len() as f32 * row_h(style)
    }
}

impl Default for LyricsView {
    fn default() -> Self {
        Self::new()
    }
}

impl LyricsView {
    pub fn new() -> Self {
        LyricsView {
            id: String::new(),
            words: None,
            state: None,
            follow: Follow::default(),
            current: None,
            at: Animated::new(0.0, role::smooth()).with_rest(0.002),
            scroll: Animated::new(0.0, role::smooth()).with_rest(0.3),
            shown: Animated::new(0.0, role::smooth()).with_rest(0.004),
            layout: None,
            stamp: u64::MAX,
            dots: 0,
            step: 17.0,
        }
    }

    /// Time passing, with the player as it is. Returns (moving, changed).
    pub fn tick(&mut self, dt: f32, n: &Now) -> (bool, bool) {
        let mut changed = false;
        let track = n.track().cloned();
        let id = track.as_ref().map(|t| t.id.clone()).unwrap_or_default();
        if id != self.id {
            self.id = id;
            self.words = None;
            self.state = None;
            self.layout = None;
            self.follow.reset();
            self.current = None;
            self.scroll.jump(0.0);
            self.shown.jump(0.0);
            self.stamp = u64::MAX;
            changed = true;
        }
        if let Some(t) = &track {
            lyrics::want(t);
        }
        let stamp = lyrics::STAMP.load(std::sync::atomic::Ordering::Relaxed);
        if stamp != self.stamp {
            self.stamp = stamp;
            let state = lyrics::held(&self.id);
            if let Some(State::Ready(words)) = &state {
                if !self.words.as_ref().is_some_and(|w| Arc::ptr_eq(w, words)) {
                    self.words = Some(words.clone());
                    self.layout = None;
                    self.follow.reset();
                    self.current = None;
                    // The words come in.
                    self.shown.jump(0.0);
                    self.shown.set(1.0);
                }
            }
            self.state = state;
            changed = true;
        }
        if let Some(lines) = self.words.as_ref().and_then(|w| w.lines()) {
            let now = self.follow.at(lines, n.position);
            if now != self.current {
                let first = self.current.is_none();
                self.current = now;
                let to = now.map_or(-1.0, |i| i as f32);
                if first {
                    // Opened, or the words just came: there at once.
                    self.at.jump(to);
                } else {
                    self.at.set(to);
                }
                changed = true;
            }
            let dots = self.pause_progress(lines, n.position).map_or(0, |p| (p * 3.0) as u32 + 1);
            if dots != self.dots {
                self.dots = dots;
                changed = true;
            }
        }
        let moving = self.at.tick(dt) | self.scroll.tick(dt) | self.shown.tick(dt);
        (moving, changed || moving)
    }

    /// Through the pause being sung, 0 to 1 (or `None`, words being sung).
    fn pause_progress(&self, lines: &[Line], t: f32) -> Option<f32> {
        let i = self.current?;
        let line = lines.get(i).filter(|l| l.text.is_empty())?;
        let end = lines.get(i + 1).map_or(line.at + 8.0, |l| l.at);
        Some(((t - line.at) / (end - line.at).max(0.5)).clamp(0.0, 0.999))
    }

    /// Plain words scrolled by hand: ↑ (`-1`) or ↓ (`1`), three rows a press.
    pub fn scroll_by(&mut self, direction: f32) {
        if matches!(self.words.as_deref(), Some(Lyrics::Plain(_))) {
            self.scroll.set((self.scroll.target() + direction * self.step * 3.0).max(0.0));
        }
    }

    fn lay_out(&mut self, g: &mut Gfx) {
        let (large, small) = voices(g);
        let Some(words) = self.words.clone() else { return };
        let key = (Arc::as_ptr(&words) as usize, large.size.to_bits(), small.size.to_bits());
        if self.layout.as_ref().is_some_and(|l| l.key == key) {
            return;
        }
        let wrap = |g: &mut Gfx, text: &str, style: TextStyle| if text.is_empty() { Vec::new() } else { balanced(g, text, style) };
        let (small_rows, large_rows) = match &*words {
            Lyrics::Synced(lines) => (lines.iter().map(|l| wrap(g, &l.text, small)).collect(), lines.iter().map(|l| wrap(g, &l.text, large)).collect()),
            Lyrics::Plain(lines) => (lines.iter().map(|l| if l.is_empty() { vec![String::new()] } else { wrap(g, l, small) }).collect(), Vec::new()),
            _ => (Vec::new(), Vec::new()),
        };
        self.layout = Some(Layout { key, small: small_rows, large: large_rows });
    }

    pub fn draw(&mut self, g: &mut Gfx, n: &Now) {
        if n.track().is_none() {
            g.empty("lyrics", "nothing playing · esc, then pick a song");
            return;
        }
        let words = self.words.clone();
        match (&self.state, words.as_deref()) {
            (_, Some(Lyrics::Synced(lines))) => {
                self.lay_out(g);
                self.draw_synced(g, lines, n.position);
            }
            (_, Some(Lyrics::Plain(_))) => {
                self.lay_out(g);
                self.draw_plain(g);
            }
            (_, Some(Lyrics::Instrumental)) => g.empty("lyrics", "an instrumental · no words to it"),
            (_, Some(Lyrics::None)) => g.empty("lyrics", "no lyrics for this song"),
            (Some(State::Offline), _) => g.empty("lyrics", "no lyrics kept for this song · they come once online"),
            (Some(State::Failed(why)), _) => g.empty("lyrics", &format!("the lyrics did not come · {why}")),
            _ => g.empty("lyrics", "looking for the lyrics…"),
        }
    }

    fn draw_synced(&mut self, g: &mut Gfx, lines: &[Line], position: f32) {
        let Some(layout) = self.layout.as_ref() else { return };
        let t = g.tokens;
        let (large, small) = voices(g);
        let top = g.top();
        let anchor = top + (H - top) * ANCHOR;
        let at = self.at.value();
        let shown = self.shown.value().clamp(0.0, 1.0);
        let rise = (1.0 - shown) * 8.0;
        // Each line's height, grown as the view comes to it, and its centre.
        let emphasis = |i: usize| (1.0 - (i as f32 - at).abs()).clamp(0.0, 1.0);
        let heights: Vec<f32> = (0..lines.len()).map(|i| {
            let e = emphasis(i);
            let grow = smoothstep(0.3, 0.7, e);
            height(&layout.small[i], small) * (1.0 - grow) + height(&layout.large[i], large) * grow
        }).collect();
        let mut centres = Vec::with_capacity(lines.len());
        let mut y = 0.0;
        for (i, h) in heights.iter().enumerate() {
            if i > 0 {
                y += heights[i - 1] * 0.5 + GAP + h * 0.5;
            }
            centres.push(y);
        }
        // The point the view is on: between two lines' centres (or before the first).
        let focus = if at < 0.0 {
            centres.first().copied().unwrap_or(0.0) + at * (heights.first().copied().unwrap_or(0.0) + GAP)
        } else {
            let i = (at.floor() as usize).min(lines.len() - 1);
            let next = centres.get(i + 1).copied().unwrap_or(centres[i] + heights[i] + GAP);
            centres[i] + (next - centres[i]) * at.fract()
        };
        let current = self.current;
        for (i, line) in lines.iter().enumerate() {
            let cy = anchor + centres[i] - focus + rise;
            let h = heights[i];
            if cy + h * 0.5 < top - 4.0 || cy - h * 0.5 > H + 4.0 {
                continue;
            }
            // Out towards the edges, the words fade.
            let edge = ((cy - top) / EDGE).min((H - cy) / EDGE).clamp(0.0, 1.0);
            let alpha = shown * edge;
            if alpha <= 0.01 {
                continue;
            }
            let e = emphasis(i);
            let past = current.is_some_and(|c| i < c);
            let quiet = if past { t.faint.mix(t.dim, 0.35) } else { t.dim };
            if line.text.is_empty() {
                let progress = if current == Some(i) { self.pause_progress(lines, position) } else { None };
                draw_dots(g, cy, e, progress, t.ink, quiet, alpha);
                continue;
            }
            // The two sizes cross over quickly, mid-way, from the line's top:
            // it reads as the line growing, not as two lines over each other.
            let grow = smoothstep(0.3, 0.7, e);
            let top_y = cy - h * 0.5;
            if grow < 0.99 {
                draw_rows(g, &layout.small[i], top_y, small, quiet, alpha * (1.0 - grow));
            }
            if grow > 0.01 {
                draw_rows(g, &layout.large[i], top_y, large, t.ink, alpha * grow);
            }
        }
    }

    fn draw_plain(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let (_, small) = voices(g);
        let top = g.top() + 4.0;
        let step = row_h(small);
        self.step = step;
        let Some(layout) = self.layout.as_ref() else { return };
        let rows: Vec<String> = layout.small.iter().flatten().cloned().collect();
        let total = rows.len() as f32 * step;
        let room = H - top - 6.0;
        let most = (total - room).max(0.0);
        if self.scroll.target() > most {
            self.scroll.set(most);
        }
        let scroll = self.scroll.value().clamp(-12.0, most + 12.0);
        let shown = self.shown.value().clamp(0.0, 1.0);
        for (k, row) in rows.iter().enumerate() {
            let cy = top + step * (k as f32 + 0.5) - scroll + (1.0 - shown) * 8.0;
            if row.is_empty() || cy < top - step || cy > H + step {
                continue;
            }
            let edge = ((cy - g.top()) / EDGE * 1.6).min((H - cy) / EDGE).clamp(0.0, 1.0);
            g.text.draw(&mut g.canvas, X, (cy + small.size * 0.36).round(), row, &small, t.ink, edge * shown);
        }
        // Where in the words, at the right edge.
        if most > 0.0 {
            let track = (H - top - 8.0).max(10.0);
            let thumb = (track * room / total).max(12.0);
            let y = top + (track - thumb) * (scroll / most).clamp(0.0, 1.0);
            g.canvas.rrect(bezel::Rect::new(W - 4.0, y, 2.5, thumb), 1.25, t.faint, 0.8 * shown);
        }
    }
}

/// A line wrapped to the width, its rows evened out ("keep me steady /
/// through the night", not a word left alone on the second).
fn balanced(g: &mut Gfx, text: &str, style: TextStyle) -> Vec<String> {
    let rows = crate::help::wrap(g, text, style, WIDTH);
    if rows.len() < 2 {
        return rows;
    }
    let (mut lo, mut hi) = (WIDTH * 0.4, WIDTH);
    for _ in 0..7 {
        let mid = (lo + hi) * 0.5;
        if crate::help::wrap(g, text, style, mid).len() == rows.len() {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    crate::help::wrap(g, text, style, hi)
}

/// A line's rows from `top`, left-aligned.
fn draw_rows(g: &mut Gfx, rows: &[String], top: f32, style: TextStyle, color: Color, alpha: f32) {
    let step = row_h(style);
    for (k, row) in rows.iter().enumerate() {
        let mid = top + step * (k as f32 + 0.5);
        g.text.draw(&mut g.canvas, X, (mid + style.size * 0.36).round(), row, &style, color, alpha);
    }
}

/// 0 below `a`, 1 above `b`, smoothly between.
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A pause in the singing: three dots, larger as it is sung, filling one
/// after another as it passes.
fn draw_dots(g: &mut Gfx, cy: f32, emphasis: f32, progress: Option<f32>, ink: Color, quiet: Color, alpha: f32) {
    let r = 2.0 + 1.2 * emphasis;
    for k in 0..3 {
        let lit = progress.map_or(0.0, |p| (p * 3.0 - k as f32).clamp(0.0, 1.0));
        let color = quiet.mix(ink, lit.max(emphasis * 0.35));
        g.canvas.circle(X + r + k as f32 * (r * 2.0 + 5.0), cy, r, color, alpha * (0.55 + 0.45 * lit.max(1.0 - emphasis)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::Track;

    fn now(id: &str, title: &str, position: f32) -> Now {
        let t = Track { id: id.into(), title: title.into(), artist: "Wren & the Tide".into(), album: "Low Water".into(), seconds: Some(214), ..Track::default() };
        Now { queue: vec![t], at: 0, phase: super::super::player::Phase::Playing, position, length: Some(214.0), loading: None, buffering: false, shuffle: false, repeat: super::super::player::Repeat::Off }
    }

    /// Ticks until the words are in (the simulator's are found on a thread).
    fn settle(view: &mut LyricsView, n: &Now) {
        for _ in 0..400 {
            view.tick(1.0 / 60.0, n);
            if view.words.is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn the_view_springs_from_line_to_line_and_glides_through_a_seek() {
        let mut view = LyricsView::new();
        let n = now("lyrics-view-test", "Harbour Lights", 15.0);
        settle(&mut view, &n);
        assert_eq!(view.current, Some(1), "the first line, after the pause");
        assert_eq!(view.at.value(), 1.0, "there at once as the words come");
        // The next line: the view moves there over frames, not at once.
        let n = now("lyrics-view-test", "Harbour Lights", 19.0);
        view.tick(1.0 / 60.0, &n);
        assert_eq!(view.current, Some(2));
        assert!(view.at.value() > 1.0 && view.at.value() < 1.5, "{}", view.at.value());
        // A seek well on: every frame between is a small step.
        let n = now("lyrics-view-test", "Harbour Lights", 95.0);
        let mut last = view.at.value();
        let mut biggest: f32 = 0.0;
        for _ in 0..90 {
            view.tick(1.0 / 60.0, &n);
            biggest = biggest.max(view.at.value() - last);
            last = view.at.value();
        }
        let to = view.current.unwrap() as f32;
        assert!((view.at.value() - to).abs() < 0.01, "arrived: {} of {to}", view.at.value());
        assert!(biggest < (to - 2.0) * 0.25, "no jump: the largest frame's step was {biggest} lines of {}", to - 2.0);
    }
}
