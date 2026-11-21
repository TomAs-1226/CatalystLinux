//! A round, then its results, on one page: the clock ends a sprint between
//! keys, so the results arrive in place rather than as a page pushed on.
//!
//! Typing: the word large, the letters typed in ink and the rest quiet,
//! a caret under the next letter gliding along; the next word below. A
//! finished word lifts away as the next rises from where it waited. A wrong
//! key shakes the word and flashes the letter it wanted. None of it holds
//! a key up: every key lands on the round at once, whatever is moving.
//!
//! Results: the speed in the display voice, accuracy, words and best,
//! rising in block by block as a page does.

use std::fmt::Write as _;

use catalyst_kit::prelude::*;
use catalyst_kit::Spring;

use crate::model::{Model, Phase, Round, Summary, Typed, PRACTICE_WORDS};
use crate::records::{Offer, Records};
use crate::words::Pack;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Typing,
    Results,
}

/// A wrong key shakes the word, lightly damped so it says no twice and is
/// still within 0.4 s. My own tuning: Detent has no preset for a shake.
fn shake_spring() -> Spring {
    Spring::stiffness_ratio(1400.0, 0.25)
}
/// The shake's kick in px/s: about 4 px each way.
const SHAKE_KICK: f32 = 170.0;
/// The results' blocks rise this far into place, each a little after the one
/// before (the kit's own rise, 10 px a block, a touch deeper here).
const RISE: f32 = 12.0;
const STAGGER: f32 = 0.1;
/// The word's size in the display voice (the text size scales it).
pub(crate) const WORD_SIZE: f32 = 34.0;
/// The speed's size on the results, before it is fitted to its tile.
const SPEED_SIZE: f32 = 56.0;

/// A finished round, with its words made once.
struct Outcome {
    summary: Summary,
    offer: Offer,
    wpm: String,
    rows: [(&'static str, String); 3],
}

impl Outcome {
    fn of(summary: Summary, offer: Offer) -> Outcome {
        let third = match summary.round {
            Round::Words10 => ("time", format!("{:.1} s", summary.elapsed_ms as f64 / 1000.0)),
            _ => ("words", summary.words.to_string()),
        };
        let best = if offer.best > 0 { format!("{} wpm", offer.best) } else { "–".to_owned() };
        Outcome { summary, offer, wpm: summary.wpm.to_string(), rows: [("accuracy", format!("{}%", summary.accuracy)), third, ("best", best)] }
    }
}

pub struct Play {
    model: Model,
    records: Records,
    stage: Stage,
    outcome: Option<Outcome>,
    /// Seconds since the page opened, as its ticks have counted them: the
    /// round's clock, so the gallery and tests keep time as frames do.
    clock: f64,
    calm: bool,
    /// The word in play: 1 where the next word waits, 0 in its place.
    arrive: Animated,
    /// The word just finished: 0 in its place, -1 gone above.
    leave: Animated,
    left: &'static str,
    /// The caret under the next letter: from the word's left, and its width.
    caret_x: Animated,
    caret_w: Animated,
    caret_placed: bool,
    shake: Animated,
    flash: Animated,
    /// Words done, for the ten-word round's meter.
    count: Animated,
    /// 0 typing, 1 results.
    view: Animated,
    /// What the stats line says before the first key.
    ready: String,
    /// Numbers written each frame, into the same strings.
    live: [String; 4],
}

impl Play {
    pub fn new(round: Round, pack: Pack, seed: u32, records: Records) -> Play {
        Play {
            model: Model::new(round, pack, seed),
            records,
            stage: Stage::Typing,
            outcome: None,
            clock: 0.0,
            calm: false,
            arrive: Animated::new(0.0, role::release()).with_rest(0.002),
            leave: Animated::new(-1.0, role::smooth()).with_rest(0.002),
            left: "",
            caret_x: Animated::new(0.0, spatial::fast()).with_rest(0.05),
            caret_w: Animated::new(0.0, spatial::default()).with_rest(0.05),
            caret_placed: false,
            shake: Animated::new(0.0, shake_spring()).with_rest(0.05),
            flash: Animated::new(0.0, role::smooth()).with_rest(0.01),
            count: Animated::new(0.0, role::settle()).with_rest(0.002),
            view: Animated::new(0.0, role::release()).with_rest(0.002),
            ready: format!("{} words · the clock starts on your first key", pack.key()),
            live: Default::default(),
        }
    }

    fn now(&self) -> u64 {
        (self.clock * 1000.0) as u64
    }

    fn springs(&mut self) -> [&mut Animated; 8] {
        [&mut self.arrive, &mut self.leave, &mut self.caret_x, &mut self.caret_w, &mut self.shake, &mut self.flash, &mut self.count, &mut self.view]
    }

    /// What a key did, set moving.
    fn react(&mut self, typed: Typed, word: &'static str) {
        match typed {
            Typed::Wrong if !self.calm => {
                self.shake.nudge(SHAKE_KICK);
                self.flash.jump(1.0);
                self.flash.set(0.0);
            }
            Typed::Word => {
                self.left = word;
                if self.calm {
                    self.leave.jump(-1.0);
                    self.arrive.jump(0.0);
                } else {
                    // The finished word leaves from wherever it was, at its own speed.
                    self.leave = self.arrive;
                    self.leave.set_with(-1.0, role::smooth());
                    self.arrive = Animated::new(1.0, role::release()).with_rest(0.002);
                    self.arrive.set(0.0);
                }
                self.caret_placed = false;
                self.shake.jump(0.0);
                self.flash.jump(0.0);
                self.count_words();
            }
            Typed::Finished => self.count_words(),
            _ => {}
        }
    }

    fn count_words(&mut self) {
        let n = self.model.words_done() as f32;
        if self.calm {
            self.count.jump(n);
        } else {
            self.count.set(n);
        }
    }

    fn view_to(&mut self, to: f32) {
        if self.calm {
            self.view.jump(to);
        } else {
            self.view.set(to);
        }
    }

    /// If the round has just ended: its speed offered to the records, and
    /// the results on their way in. True if it had.
    fn settle(&mut self) -> bool {
        if self.stage != Stage::Typing || self.model.phase() != Phase::Done {
            return false;
        }
        let summary = self.model.summary();
        let offer = self.records.offer(summary.round, summary.pack, summary.wpm);
        self.outcome = Some(Outcome::of(summary, offer));
        self.stage = Stage::Results;
        self.view_to(1.0);
        true
    }

    /// The same round again, its words dealt on.
    fn again(&mut self) {
        self.model.restart();
        self.stage = Stage::Typing;
        self.caret_placed = false;
        self.left = "";
        self.count.jump(0.0);
        self.arrive.jump(0.0);
        self.leave.jump(-1.0);
        self.shake.jump(0.0);
        self.flash.jump(0.0);
        self.view_to(0.0);
    }

    /// The round: clock, word, next word and live numbers, at `alpha`,
    /// lifted by `lift` px as it gives way to the results.
    fn draw_typing(&mut self, ui: &mut Ui, alpha: f32, lift: f32) {
        let (t, ty) = (ui.tokens, ui.ty);
        let (lab, cap) = (ty.label, ty.caption);
        let now = self.now();
        let m = self.model;
        let running = m.phase() == Phase::Running;
        let (top, bottom) = (ui.top(), ui.bottom());

        // The time left (or the words done) as a meter, its number beside it.
        let cy = top + lab.size * 0.5 + 1.0 - lift;
        let room = ui.measure("60 s", lab).max(ui.measure("10/10", lab));
        let track = Rect::new(PAD + 2.0, cy - 3.0, W - 2.0 * PAD - 4.0 - room - 10.0, 6.0);
        let label = &mut self.live[0];
        label.clear();
        let (fraction, ink) = match m.round() {
            Round::Words10 => {
                let _ = write!(label, "{}/{}", m.words_done(), PRACTICE_WORDS);
                (self.count.value() / PRACTICE_WORDS as f32, if running { t.ink } else { t.dim })
            }
            round => {
                let left = m.remaining_ms(now);
                let _ = write!(label, "{} s", left.div_ceil(1000));
                let ink = match running {
                    true if left <= 5_000 => t.amber_ink,
                    true => t.ink,
                    false => t.dim,
                };
                (left as f32 / round.limit_ms() as f32, ink)
            }
        };
        meter(ui, track, fraction, t.ice, alpha);
        let w = ui.measure(label, lab);
        words(ui, W - PAD - 2.0 - w, baseline(cy, lab.size), label, lab, ink, alpha);

        // Where the word and the next one go, between the meter and the numbers.
        let big = ty.display(WORD_SIZE);
        let stats_cy = bottom - lab.size * 0.5 - 1.0 - lift;
        let (from, to) = (track.bottom() + 4.0, stats_cy - lab.size * 0.5 - 4.0);
        let block = big.size * 0.98 + 8.5 + lab.size;
        let y0 = ((from + to - block) * 0.5).round();
        let word_cy = y0 + big.size * 0.36;
        let next_cy = y0 + big.size * 0.98 + 8.5 + lab.size * 0.5;
        let travel = next_cy - word_cy;

        // The word just finished, lifting away.
        let gone = self.leave.value();
        if gone > -0.999 && !self.left.is_empty() {
            let a = alpha * (1.0 + gone * 1.4).clamp(0.0, 1.0);
            let w = ui.measure(self.left, big);
            ui.text_alpha(((W - w) * 0.5).round(), word_cy + gone * travel * 0.7, self.left, big, t.ink, a);
        }

        // The word: typed in ink, the letter wanted next marked, the rest quiet.
        let rise = self.arrive.value();
        let wa = alpha * (1.0 - rise * 1.25).clamp(0.0, 1.0);
        let wy = word_cy + rise * travel;
        let word = m.word();
        let typed = m.typed().min(word.len());
        let end = (typed + 1).min(word.len());
        let (head, cur, tail) = (&word[..typed], &word[typed..end], &word[end..]);
        let tracking = big.tracking * big.size;
        let x0 = ((W - ui.measure(word, big)) * 0.5).round() + self.shake.value();
        let head_w = if head.is_empty() { 0.0 } else { ui.measure(head, big) + tracking };
        let cur_w = ui.measure(cur, big);
        let wrong = !m.last_correct();
        let fault = ui.fault();
        let flash = self.flash.value();
        if flash > 0.01 && !cur.is_empty() {
            let r = Rect::new(x0 + head_w - 3.0, wy - big.size * 0.44, cur_w + 6.0, big.size * 0.9);
            let radius = ui.radius(6.0);
            fill(ui, r, radius, fault, 0.3 * flash * wa);
        }
        ui.text_alpha(x0, wy, head, big, t.ink, wa);
        ui.text_alpha(x0 + head_w, wy, cur, big, if wrong { fault } else { t.dim }, wa);
        ui.text_alpha(x0 + head_w + cur_w + tracking, wy, tail, big, t.dim, wa);

        // The caret, under the letter wanted next, gliding along the word.
        if !cur.is_empty() && self.stage == Stage::Typing {
            let (cx, cw) = (head_w, cur_w);
            if !self.caret_placed || self.calm {
                self.caret_x.jump(cx);
                self.caret_w.jump(cw);
                self.caret_placed = true;
            } else if self.caret_x.target() != cx || self.caret_w.target() != cw {
                self.caret_x.set(cx);
                self.caret_w.set(cw);
                ui.started_moving();
            }
            let y = baseline(wy, big.size) + (big.size * 0.24).round();
            let r = Rect::new(x0 + self.caret_x.value(), y, self.caret_w.value().max(2.0), 2.5);
            let ink = if wrong { fault } else { ui.ring_color() };
            fill(ui, r, 1.25, ink, wa);
        }

        // The next word, quietly, where it waits to rise.
        let pa = alpha * (1.0 - rise).clamp(0.0, 1.0);
        let last = m.round() == Round::Words10 && m.words_done() + 1 >= PRACTICE_WORDS;
        let (tag, next) = if last { ("last word", "") } else { ("next", m.next()) };
        let (tw, nw) = (ui.measure(tag, cap), if next.is_empty() { 0.0 } else { ui.measure(next, lab) + 6.0 });
        let x = ((W - tw - nw) * 0.5).round();
        let b = baseline(next_cy, lab.size);
        words(ui, x, b, tag, cap, t.dim, pa);
        words(ui, x + tw + 6.0, b, next, lab, t.dim, pa);

        // The live numbers, or how to begin.
        if m.phase() == Phase::Ready {
            let choices = [self.ready.as_str(), "the clock starts on your first key", "starts on your first key"];
            let line = choices.into_iter().find(|s| ui.measure(s, cap) <= W - 2.0 * PAD).unwrap_or("");
            let w = ui.measure(line, cap);
            words(ui, ((W - w) * 0.5).round(), baseline(stats_cy, cap.size), line, cap, t.dim, alpha);
        } else {
            let [_, wpm, accuracy, third] = &mut self.live;
            wpm.clear();
            let _ = write!(wpm, "{}", m.wpm(now));
            accuracy.clear();
            let _ = write!(accuracy, "{}%", m.accuracy());
            third.clear();
            let unit = match m.round() {
                Round::Words10 => {
                    let _ = write!(third, "{}", m.elapsed_ms(now) / 1000);
                    "s"
                }
                _ => {
                    let _ = write!(third, "{}", m.words_done());
                    if m.words_done() == 1 { "word" } else { "words" }
                }
            };
            let b = baseline(stats_cy, lab.size);
            stat(ui, Align::Left, PAD + 4.0, b, wpm, "wpm", alpha);
            stat(ui, Align::Center, W * 0.5, b, accuracy, "accuracy", alpha);
            stat(ui, Align::Right, W - PAD - 4.0, b, third, unit, alpha);
        }
    }

    /// The results, block by block, `p` of the way in.
    fn draw_results(&self, ui: &mut Ui, p: f32) {
        let Some(o) = &self.outcome else { return };
        let (t, ty) = (ui.tokens, ui.ty);
        let (lab, cap) = (ty.label, ty.caption);
        let (top, bottom) = (ui.top(), ui.bottom());
        let reveal = |i: usize| {
            let q = ((p - i as f32 * STAGGER) / (1.0 - 3.0 * STAGGER)).max(0.0);
            ((1.0 - q) * RISE, q.min(1.0))
        };
        let tile_w = ((W - 2.0 * PAD - GAP) * 0.5).floor();
        let row_h = ((bottom - top - 2.0 * GAP) / 3.0).floor();
        let tall = 3.0 * row_h + 2.0 * GAP;

        // The speed, in the display voice; in the signal when it is a new best.
        let (dy, a) = reveal(0);
        if a > 0.0 {
            let r = Rect::new(PAD, top + dy, tile_w, tall);
            let radius = ui.tile_radius();
            fill(ui, r, radius, t.surface1, a);
            words(ui, r.x + 9.0, baseline(r.y + 6.0 + cap.size * 0.5, cap.size), "words a minute", cap, t.dim, a);
            let foot = baseline(r.bottom() - 8.0 - lab.size * 0.5, lab.size);
            if o.offer.new_best {
                let size = lab.size + 3.0;
                let (ink, said) = if o.offer.unsaved { (ui.fault(), "new best, not kept") } else { (t.signal, "new best") };
                ui.icon_alpha("emoji_events", true, r.x + 9.0 + size * 0.5, foot - lab.size * 0.36, size, ink, a);
                words(ui, r.x + 13.0 + size, foot, said, lab, t.ink, a);
            } else {
                let round = o.summary.round.short();
                let pack = o.summary.pack.key();
                let room = r.w - 18.0;
                let x = r.x + 9.0;
                let w = ui.measure(round, cap) + 4.0;
                let dot_w = ui.measure("·", cap) + 4.0;
                if w + dot_w + ui.measure(pack, cap) <= room {
                    words(ui, x, foot, round, cap, t.dim, a);
                    words(ui, x + w, foot, "·", cap, t.faint, a);
                    words(ui, x + w + dot_w, foot, pack, cap, t.dim, a);
                } else {
                    words(ui, x, foot, pack, cap, t.dim, a);
                }
            }
            // As large as the tile lets it be.
            let unit_w = ui.measure("wpm", cap);
            let mut size = SPEED_SIZE;
            let mut style = ty.display(size);
            while size > 24.0 && ui.measure(&o.wpm, style) + 4.0 + unit_w > r.w - 18.0 {
                size -= 2.0;
                style = ty.display(size);
            }
            let b = (foot - lab.size - 10.0).round();
            let ink = if o.offer.new_best { t.signal } else { t.ink };
            let w = words(ui, r.x + 8.0, b, &o.wpm, style, ink, a);
            words(ui, r.x + 12.0 + w, b, "wpm", cap, t.dim, a);
        }

        // Accuracy, words (or time), best.
        let x = PAD + tile_w + GAP;
        for (i, (label, value)) in o.rows.iter().enumerate() {
            let (dy, a) = reveal(i + 1);
            if a <= 0.0 {
                continue;
            }
            let r = Rect::new(x, top + i as f32 * (row_h + GAP) + dy, W - PAD - x, row_h);
            let radius = ui.tile_radius().min(row_h * 0.5);
            fill(ui, r, radius, t.surface1, a);
            words(ui, r.x + 10.0, baseline(r.cy(), lab.size), label, lab, t.dim, a);
            let w = ui.measure(value, ty.name);
            words(ui, r.right() - 10.0 - w, baseline(r.cy(), ty.name.size), value, ty.name, t.ink, a);
        }
    }
}

impl Page for Play {
    fn title(&self) -> &str {
        match self.stage {
            Stage::Typing => self.model.round().title(),
            Stage::Results => "Results",
        }
    }

    /// Letters are letters while the round is on; on the results, Backspace
    /// is Back again.
    fn wants_text(&self) -> bool {
        self.stage == Stage::Typing
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        self.calm = cx.calm();
        if self.stage == Stage::Results {
            return match key {
                Key::Back => Flow::Pop,
                Key::Enter => {
                    self.again();
                    Flow::Stay
                }
                _ => Flow::Stay,
            };
        }
        let now = self.now();
        match key {
            Key::Back => return Flow::Pop,
            Key::Backspace => {
                self.model.backspace(now);
            }
            Key::Char(c) => {
                let word = self.model.word();
                let typed = self.model.type_char(c, now);
                self.react(typed, word);
            }
            _ => {}
        }
        self.settle();
        Flow::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.clock += dt as f64;
        let mut moving = false;
        for spring in self.springs() {
            moving |= spring.tick(dt);
        }
        // The clock is shown running: every frame, while it runs.
        if self.stage == Stage::Typing && self.model.phase() == Phase::Running {
            let now = self.now();
            self.model.tick(now);
            moving = true;
        }
        self.settle() | moving
    }

    fn draw(&mut self, ui: &mut Ui) {
        self.calm = ui.calm();
        let p = self.view.value();
        if p < 0.999 {
            self.draw_typing(ui, (1.0 - p * 1.6).clamp(0.0, 1.0), p.max(0.0) * 10.0);
        }
        if p > 0.001 {
            self.draw_results(ui, p);
        }
        let hints: &[&str] = match (self.stage, self.model.phase()) {
            (Stage::Results, _) => &["enter: again · esc: rounds", "enter · esc"],
            (_, Phase::Ready) => &["esc: back to the rounds", "esc: back"],
            _ => &["backspace: fix a letter · esc: stop", "esc: stop"],
        };
        ui.hint_any(hints);
    }
}

/// The baseline that centres a line of `size` px type on `cy`, as the kit places it.
fn baseline(cy: f32, size: f32) -> f32 {
    (cy + size * 0.36).round()
}

/// Words from `x` on a baseline, at an opacity. Returns their width.
fn words(ui: &mut Ui, x: f32, baseline: f32, s: &str, style: TextStyle, color: Color, alpha: f32) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    let x = x + ui.ox;
    ui.text.draw(&mut ui.canvas, x, baseline, s, &style, color, alpha)
}

/// A surface at an opacity.
fn fill(ui: &mut Ui, r: Rect, radius: f32, color: Color, alpha: f32) {
    if alpha >= 0.999 {
        ui.surface(r, radius, color);
    } else if alpha > 0.0 {
        ui.canvas.rrect(r.offset(ui.ox, 0.0), radius, color, alpha);
    }
}

/// The kit's meter, at an opacity: the fill takes the track's outer corners.
fn meter(ui: &mut Ui, r: Rect, fraction: f32, color: Color, alpha: f32) {
    if alpha >= 0.999 {
        return ui.meter(r, fraction, color);
    }
    let track = r.offset(ui.ox, 0.0);
    let radius = r.h * 0.5;
    let well = ui.tokens.meter;
    ui.canvas.rrect(track, radius, well, alpha);
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        let w = (track.w * fraction).max(r.h * 0.6);
        ui.canvas.rrect_clipped(Rect::new(track.x, track.y, w, track.h), 2.0, track, radius, color, alpha);
    }
}

/// A live number in mono with its unit after it, quieter, placed by `align`.
fn stat(ui: &mut Ui, align: Align, x: f32, baseline: f32, value: &str, unit: &str, alpha: f32) {
    let (t, ty) = (ui.tokens, ui.ty);
    let vw = ui.measure(value, ty.label);
    let total = vw + 4.0 + ui.measure(unit, ty.caption);
    let left = match align {
        Align::Left => x,
        Align::Center => x - total * 0.5,
        Align::Right => x - total,
    }
    .round();
    words(ui, left, baseline, value, ty.label, t.ink, alpha);
    words(ui, left + vw + 4.0, baseline, unit, ty.caption, t.dim, alpha);
}
