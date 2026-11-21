//! Recent apps: the last few apps left, kept alive just as they were (their
//! pages, their place in a list, a terminal's session), and the switcher that
//! shows them as cards: Android's recents, in Bezel's terms. Each card is a
//! tonal panel holding the app's last picture; the screen behind dims; the
//! card in front wears the focus ring.
//!
//! Ctrl-Tab (or Recents in the control centre) brings the switcher up over
//! any screen, a terminal included: no program in a terminal can be sent
//! Ctrl-Tab, so it takes nothing from one. The app before the one in front is
//! offered first, so Ctrl-Tab then Enter goes back and forth between two.
//! Left and Right choose, Enter brings an app back growing out of its card,
//! Up (or Backspace, the handheld's Del) lets one go, Esc puts the switcher
//! away. An app opened again from Home comes back as it was, too.

use bezel::canvas::sd_rrect;
use bezel::simd::mix_px;
use bezel::{Canvas, Rect};
use detent::{role, Animated};

use crate::gfx::{Gfx, H, PAD, W};
use crate::platform::Key;
use crate::shell::{Path, Screen};

/// How many apps are kept besides the one in front.
pub const KEEP: usize = 6;
/// A card's picture: the panel at 0.425 (so a card is 39 kB, not 218).
pub const CARD_W: usize = 136;
pub const CARD_H: usize = 72;
const CARD_GAP: f32 = 14.0;
const CARD_RADIUS: f32 = 10.0;
/// Where the cards' tops rest.
const CARD_Y: f32 = 46.0;
/// How far the wallpaper behind the cards sinks, so they stand off it.
const DIM: f32 = 0.25;

/// An app kept: its pages (its own first, each with the path it arrived
/// by) and its last picture, small.
pub struct Recent {
    pub stack: Vec<(Box<dyn Screen>, Path)>,
    pub thumb: Vec<u32>,
}

impl Recent {
    pub fn title(&self) -> &str {
        self.stack.first().map_or("", |(s, _)| s.title())
    }
}

/// The apps kept, the most recently left first.
#[derive(Default)]
pub struct Recents {
    list: Vec<Recent>,
}

impl Recents {
    /// An app left: kept with its picture (the panel's), one card an app.
    /// Beyond `KEEP`, the oldest is let go.
    pub fn remember(&mut self, stack: Vec<(Box<dyn Screen>, Path)>, picture: Option<&[u32]>) {
        let thumb = picture.map(thumbnail).unwrap_or_else(|| vec![0; CARD_W * CARD_H]);
        self.keep(stack, thumb);
    }

    /// As `remember`, with the card's picture already made.
    pub fn keep(&mut self, stack: Vec<(Box<dyn Screen>, Path)>, thumb: Vec<u32>) {
        let Some((first, _)) = stack.first() else { return };
        let title = first.title().to_owned();
        self.list.retain(|r| r.title() != title);
        self.list.insert(0, Recent { stack, thumb });
        self.list.truncate(KEEP);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Recent> {
        self.list.iter()
    }

    /// Where an app is among them, by its name.
    pub fn find(&self, title: &str) -> Option<usize> {
        self.list.iter().position(|r| r.title() == title)
    }

    /// Take one back out, to show it again.
    pub fn take(&mut self, at: usize) -> Option<Recent> {
        (at < self.list.len()).then(|| self.list.remove(at))
    }

    /// Let one go: its pages are closed for good.
    pub fn forget(&mut self, at: usize) {
        if at < self.list.len() {
            self.list.remove(at);
        }
    }
}

/// The panel's picture made small for a card: each card pixel the average
/// of the panel pixels under it.
pub fn thumbnail(px: &[u32]) -> Vec<u32> {
    let (w, h) = (W as usize, H as usize);
    if px.len() != w * h {
        return vec![0; CARD_W * CARD_H];
    }
    let mut out = Vec::with_capacity(CARD_W * CARD_H);
    for ty in 0..CARD_H {
        let (y0, y1) = (ty * h / CARD_H, ((ty + 1) * h / CARD_H).max(ty * h / CARD_H + 1));
        for tx in 0..CARD_W {
            let (x0, x1) = (tx * w / CARD_W, ((tx + 1) * w / CARD_W).max(tx * w / CARD_W + 1));
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                for &p in &px[y * w + x0..y * w + x1] {
                    r += (p >> 16) & 0xff;
                    g += (p >> 8) & 0xff;
                    b += p & 0xff;
                    n += 1;
                }
            }
            out.push(((r / n) << 16) | ((g / n) << 8) | (b / n));
        }
    }
    out
}

/// A card's icon: the built-in app's own, else a generic one.
pub fn icon_for(title: &str) -> &'static str {
    crate::screens::builtin_apps().find(|(_, _, name)| *name == title).map_or("apps", |(_, icon, _)| icon)
}

/// One card in the switcher.
pub struct Card {
    pub title: String,
    pub icon: &'static str,
    pub thumb: Vec<u32>,
    /// The app in front when the switcher came up.
    pub current: bool,
}

/// What a key in the switcher asks for.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Bring this card's app to the front.
    Open(usize),
    /// Let this card's app go.
    Dismiss(usize),
}

/// The switcher: the cards in a row, the chosen one in the middle.
pub struct Switcher {
    pub cards: Vec<Card>,
    /// Each card's place along the row (in cards), gliding to its index: a
    /// card let go of leaves a gap the others close.
    slots: Vec<Animated>,
    focus: usize,
    /// Which card is in the middle, gliding between them (in cards).
    scroll: Animated,
    /// Arriving (to 1) and going (to 0).
    shown: Animated,
    leaving: bool,
    /// Cards let go of, rising away as they fade: the card, where it was
    /// across, and how far gone.
    flying: Vec<(Card, f32, Animated)>,
    calm: bool,
}

impl Switcher {
    pub fn new(cards: Vec<Card>, calm: bool) -> Self {
        // The app before the one in front first: Ctrl-Tab, Enter goes back to it.
        let focus = if cards.first().is_some_and(|c| c.current) && cards.len() > 1 { 1 } else { 0 };
        let mut scroll = Animated::new(focus as f32, role::detent()).with_rest(0.002);
        scroll.jump(focus as f32);
        let mut shown = Animated::new(0.0, role::release()).with_rest(0.003);
        shown.set(1.0);
        if calm {
            shown.jump(1.0);
        }
        let slots = (0..cards.len()).map(|i| Animated::new(i as f32, role::detent()).with_rest(0.002)).collect();
        Switcher { cards, slots, focus, scroll, shown, leaving: false, flying: Vec::new(), calm }
    }

    #[cfg(test)]
    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn leaving(&self) -> bool {
        self.leaving
    }

    /// Put away, sinking as it fades.
    pub fn close(&mut self) {
        self.leaving = true;
        self.shown.set_with(0.0, role::smooth());
        if self.calm {
            self.shown.jump(0.0);
        }
    }

    pub fn gone(&self) -> bool {
        self.leaving && self.shown.value() <= 0.003
    }

    /// Brought up again while `old` was on its way out: the cards rise
    /// back from where those had sunk to, at their speed, not from nothing.
    pub fn carry_on_from(&mut self, old: &Switcher) {
        if old.leaving && !self.calm {
            self.shown = old.shown;
            self.shown.set_with(1.0, role::release());
        }
    }

    fn glide(&mut self) {
        if self.calm {
            self.scroll.jump(self.focus as f32);
        } else {
            self.scroll.set(self.focus as f32);
        }
    }

    pub fn key(&mut self, key: Key) -> Action {
        if self.leaving {
            return Action::Stay;
        }
        let count = self.cards.len();
        match key {
            Key::Back | Key::Leave | Key::Ctrl('q') | Key::Down => return Action::Close,
            Key::Enter | Key::Char(' ') if count > 0 => return Action::Open(self.focus),
            Key::Enter | Key::Char(' ') => return Action::Close,
            Key::Up | Key::Backspace if count > 0 => return Action::Dismiss(self.focus),
            Key::Left if self.focus > 0 => self.focus -= 1,
            Key::Right if self.focus + 1 < count => self.focus += 1,
            // Against an end: the row gives a little and springs back.
            Key::Left | Key::Right if !self.calm && count > 0 => self.scroll.nudge(if key == Key::Left { -3.0 } else { 3.0 }),
            // Tab and Ctrl-Tab go round.
            Key::Tab | Key::Ctrl('\t') if count > 0 => self.focus = (self.focus + 1) % count,
            _ => return Action::Stay,
        }
        self.glide();
        Action::Stay
    }

    /// A card let go of: off the row, rising away; the rest close up, and
    /// the ring stays on the card it was on (or takes the next).
    pub fn remove(&mut self, at: usize) {
        if at >= self.cards.len() {
            return;
        }
        let x = self.card_rect(at).x;
        let card = self.cards.remove(at);
        self.slots.remove(at);
        if !self.calm {
            let mut away = Animated::new(0.0, role::smooth()).with_rest(0.003);
            away.set(1.0);
            self.flying.push((card, x, away));
        }
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if self.calm {
                slot.jump(i as f32);
            } else {
                slot.set(i as f32);
            }
        }
        if at < self.focus {
            self.focus -= 1;
        }
        self.focus = self.focus.min(self.cards.len().saturating_sub(1));
        self.glide();
    }

    /// Advance its springs; true while anything still moves.
    pub fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.shown.tick(dt) | self.scroll.tick(dt);
        for slot in &mut self.slots {
            moving |= slot.tick(dt);
        }
        for (_, _, away) in &mut self.flying {
            moving |= away.tick(dt);
        }
        self.flying.retain(|(_, _, away)| away.value() < 0.995);
        moving
    }

    /// Where a card is drawn just now: the chosen one in the middle, the
    /// others beside it, all rising into place as the switcher arrives.
    pub fn card_rect(&self, at: usize) -> Rect {
        let shown = self.shown.value().clamp(0.0, 1.2);
        let slot = self.slots.get(at).map_or(at as f32, Animated::value);
        let x = (W - CARD_W as f32) * 0.5 + (slot - self.scroll.value()) * (CARD_W as f32 + CARD_GAP);
        Rect::new(x, CARD_Y + (1.0 - shown) * 22.0, CARD_W as f32, CARD_H as f32)
    }

    /// The switcher over `wall`, the wallpaper alone: `front`, the screen it
    /// came up over, fades into it as the cards arrive (and back as they go).
    pub fn draw(&mut self, g: &mut Gfx, wall: &[u32], front: &[u32]) {
        let ty = g.ty;
        let t = g.tokens;
        let shown = self.shown.value().clamp(0.0, 1.0);
        let px = &mut g.canvas.px;
        if front.len() == px.len() && wall.len() == px.len() {
            px.copy_from_slice(front);
            bezel::simd::mix_into(px, Some(wall), (shown * 256.0) as u32);
        }
        bezel::simd::mix_into(px, None, (DIM * shown * 256.0) as u32);
        g.text_alpha(PAD + 4.0, 14.0, "recent apps", ty.caption, t.dim, shown);
        if self.cards.is_empty() && self.flying.is_empty() {
            g.icon_alpha("history", false, W * 0.5, H * 0.5 - 12.0, 26.0, t.dim, shown);
            let style = ty.caption;
            let w = g.measure("no recent apps", style);
            g.text_alpha(((W - w) * 0.5).round(), H * 0.5 + 14.0, "no recent apps", style, t.dim, shown);
        }
        for at in 0..self.cards.len() {
            let r = self.card_rect(at);
            if r.right() < -4.0 || r.x > W + 4.0 {
                continue;
            }
            let focused = at == self.focus;
            Self::draw_card(g, &self.cards[at], r, shown, focused);
        }
        for (card, x, away) in &self.flying {
            let v = away.value().clamp(0.0, 1.0);
            let r = Rect::new(*x, CARD_Y - v * 46.0, CARD_W as f32, CARD_H as f32);
            Self::draw_card(g, card, r, shown * (1.0 - v), false);
        }
        if !self.cards.is_empty() {
            let hint = "enter: open · ↑: let go · esc: back";
            let w = g.measure(hint, ty.caption);
            g.text_alpha(((W - w) * 0.5).round(), H - 2.0 - ty.caption.size * 0.5, hint, ty.caption, t.dim, shown);
        }
    }

    fn draw_card(g: &mut Gfx, card: &Card, r: Rect, alpha: f32, focused: bool) {
        if alpha <= 0.01 {
            return;
        }
        let (ty, t) = (g.ty, g.tokens);
        let ink = if focused { t.ink } else { t.dim };
        g.shadow(r, CARD_RADIUS, alpha);
        blit_rounded(&mut g.canvas, &card.thumb, r, CARD_RADIUS, alpha);
        // The cards beside the chosen one sit back a little.
        if !focused {
            g.canvas.rrect(r, CARD_RADIUS, t.ground, 0.35 * alpha);
        }
        // Its app above it: icon and name.
        let label_y = r.y - 9.0;
        g.icon_alpha(card.icon, focused, r.x + 6.0, label_y, 12.0, ink, alpha);
        let name = if card.current { format!("{} · now", card.title.to_lowercase()) } else { card.title.to_lowercase() };
        let name = g.text.fit(&name, &ty.caption, r.w - 16.0);
        g.text_alpha(r.x + 15.0, label_y, &name, ty.caption, ink, alpha);
        if focused {
            let ring = g.ring_color();
            g.canvas.stroke_rrect(r.inset(-2.5), CARD_RADIUS + 2.5, 1.5, ring, alpha);
        }
    }
}

/// A card's picture, its corners rounded, at an opacity.
fn blit_rounded(canvas: &mut Canvas, thumb: &[u32], r: Rect, radius: f32, alpha: f32) {
    if thumb.len() != CARD_W * CARD_H {
        return;
    }
    let (x0, y0) = (r.x.round() as i32, r.y.round() as i32);
    let (cw, ch) = (canvas.w, canvas.h);
    let rect = Rect::new(x0 as f32, y0 as f32, CARD_W as f32, CARD_H as f32);
    let full = (alpha.clamp(0.0, 1.0) * 256.0) as u32;
    for ty in 0..CARD_H as i32 {
        let y = y0 + ty;
        if y < 0 || y >= ch {
            continue;
        }
        let near_y = ty < radius as i32 + 1 || ty >= CARD_H as i32 - radius as i32 - 1;
        for tx in 0..CARD_W as i32 {
            let x = x0 + tx;
            if x < 0 || x >= cw {
                continue;
            }
            let near_x = tx < radius as i32 + 1 || tx >= CARD_W as i32 - radius as i32 - 1;
            let cover = if near_x && near_y {
                let d = sd_rrect(x as f32 + 0.5, y as f32 + 0.5, &rect, radius);
                ((0.5 - d).clamp(0.0, 1.0) * full as f32) as u32
            } else {
                full
            };
            if cover == 0 {
                continue;
            }
            let at = (y * cw + x) as usize;
            canvas.px[at] = mix_px(canvas.px[at], thumb[(ty as usize) * CARD_W + tx as usize], cover);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::Robot;
    use crate::shell::{Cx, Nav};

    fn card(title: &str, current: bool) -> Card {
        Card { title: title.into(), icon: icon_for(title), thumb: vec![0x336699; CARD_W * CARD_H], current }
    }

    /// A screen with a name and nothing else.
    struct Named(&'static str);

    impl Screen for Named {
        fn title(&self) -> &str {
            self.0
        }
        fn key(&mut self, _: Key, _: &mut Cx) -> Nav {
            Nav::Stay
        }
        fn draw(&mut self, _: &mut Gfx, _: &Robot) {}
    }

    #[test]
    fn the_app_before_the_one_in_front_comes_first() {
        let s = Switcher::new(vec![card("Logs", true), card("Robot", false), card("Files", false)], true);
        assert_eq!(s.focus(), 1);
        let s = Switcher::new(vec![card("Robot", false), card("Files", false)], true);
        assert_eq!(s.focus(), 0, "from Home, the last app left");
    }

    #[test]
    fn keys_choose_open_let_go_and_put_it_away() {
        let mut s = Switcher::new(vec![card("Logs", true), card("Robot", false), card("Files", false)], true);
        assert_eq!(s.key(Key::Right), Action::Stay);
        assert_eq!(s.focus(), 2);
        assert_eq!(s.key(Key::Right), Action::Stay, "against the end");
        assert_eq!(s.focus(), 2);
        assert_eq!(s.key(Key::Ctrl('\t')), Action::Stay);
        assert_eq!(s.focus(), 0, "ctrl-tab goes round");
        assert_eq!(s.key(Key::Enter), Action::Open(0));
        assert_eq!(s.key(Key::Up), Action::Dismiss(0));
        s.remove(2);
        assert_eq!(s.cards.len(), 2);
        assert_eq!(s.key(Key::Back), Action::Close);
        let mut empty = Switcher::new(Vec::new(), true);
        assert_eq!(empty.key(Key::Enter), Action::Close);
        assert_eq!(empty.key(Key::Up), Action::Stay);
    }

    #[test]
    fn recents_keep_one_card_an_app_and_only_a_few() {
        let mut r = Recents::default();
        for name in ["Logs", "Robot", "Files", "Git", "Loop", "Tables", "Health", "Logs"] {
            r.remember(vec![(Box::new(Named(name)) as Box<dyn Screen>, Path::Slide)], None);
        }
        let titles: Vec<&str> = r.iter().map(Recent::title).collect();
        assert_eq!(titles, ["Logs", "Health", "Tables", "Loop", "Git", "Files"], "the newest first, one card an app, six kept");
        assert_eq!(r.find("Git"), Some(4));
        let git = r.take(4).unwrap();
        assert_eq!(git.title(), "Git");
        r.forget(0);
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn a_card_let_go_of_leaves_a_gap_the_others_close() {
        let mut s = Switcher::new(vec![card("Logs", true), card("Robot", false), card("Files", false)], false);
        // Let go of the middle one: Files glides from the third place to the second.
        let before = s.card_rect(2).x;
        s.remove(1);
        assert_eq!(s.cards.len(), 2);
        assert_eq!(s.focus(), 1, "the ring takes the next card");
        assert!((s.card_rect(1).x - before).abs() < 1.0, "no jump");
        for _ in 0..60 {
            s.tick(1.0 / 60.0);
        }
        assert!((s.card_rect(1).x - (W - CARD_W as f32) * 0.5).abs() < 1.0, "and comes to the middle");
    }

    #[test]
    fn a_thumbnail_keeps_what_the_panel_showed() {
        let mut px = vec![0u32; (W * H) as usize];
        // The left half white: so is the card's.
        for y in 0..H as usize {
            for x in 0..(W as usize / 2) {
                px[y * W as usize + x] = 0xffffff;
            }
        }
        let t = thumbnail(&px);
        assert_eq!(t.len(), CARD_W * CARD_H);
        assert_eq!(t[10 * CARD_W + 5], 0xffffff);
        assert_eq!(t[10 * CARD_W + CARD_W - 5], 0);
    }

    #[test]
    fn every_card_name_and_the_hint_fit() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            assert!(g.measure("enter: open · ↑: let go · esc: back", ty.caption) <= W - 2.0 * PAD);
            for (_, _, name) in crate::screens::builtin_apps() {
                // A long name is cut with an ellipsis, never run over the card.
                let fitted = g.text.fit(&format!("{} · now", name.to_lowercase()), &ty.caption, CARD_W as f32 - 16.0);
                assert!(g.measure(&fitted, ty.caption) <= CARD_W as f32 - 16.0);
            }
        }
    }
}
