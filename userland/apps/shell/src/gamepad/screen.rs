//! Gamepad's four pages, side by side: the controller live, every input it
//! has, its check, and the checks kept (the one to bring first).

use std::f32::consts::TAU;
use std::path::PathBuf;

use bezel::{Align, Color, Rect};
use detent::{role, Animated};

use super::evdev::{self, *};
use super::health::{self, Rest};
use super::pads::Pads;
use super::{Record, Usb};
use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field as Line, Typed};

const PAGES: usize = 4;
const LIVE: usize = 0;
const INPUTS: usize = 1;
const CHECK: usize = 2;
const KEPT: usize = 3;
/// No more than a frame a beat (the handheld's 40 a second).
const FRAME: f32 = 1.0 / 40.0;
/// The check: hands off for this long, then sweeping for at least, at most.
const REST_S: f32 = 2.0;
const SWEEP_MIN_S: f32 = 2.0;
const SWEEP_MAX_S: f32 = 12.0;
const RUMBLE_MS: u16 = 600;
/// Button chips on a row of the inputs page.
const CHIPS: usize = 4;

/// What was drawn of a controller: how many there are, its axes (value
/// and range), its buttons and its report rate.
#[derive(Default, PartialEq)]
struct Snapshot {
    count: usize,
    axes: Vec<(i32, i32)>,
    buttons: Vec<bool>,
    rate: Option<i32>,
}

enum Check {
    Idle,
    Rest { t: f32, samples: [Vec<(f32, f32)>; 2] },
    Sweep { t: f32, rest: [Rest; 2], samples: [Vec<(f32, f32)>; 2], covered: [[bool; health::SECTORS]; 2], times: Vec<i64> },
    Done(Record),
}

pub struct Gamepad {
    pads: Pads,
    /// The controller shown, of those plugged in.
    pad: usize,
    page: usize,
    turn: Animated,
    usb: Usb,
    usb_for: f32,
    check: Check,
    kept: Vec<(PathBuf, Record)>,
    inputs: ListState,
    results: ListState,
    kept_list: ListState,
    /// Enter on a check: the name it is kept under, being typed.
    naming: Option<Line>,
    forgetting: Option<usize>,
    /// What was last drawn, to draw again only when that would change.
    drawn: Snapshot,
    pending: bool,
    since_draw: f32,
    /// Since the controller last reported.
    quiet_for: f32,
    /// The rumble's word shows this much longer.
    rumble_for: f32,
}

impl Gamepad {
    pub fn new() -> Self {
        Gamepad::with(Pads::open())
    }

    pub fn with(pads: Pads) -> Self {
        Gamepad {
            pads,
            pad: 0,
            page: LIVE,
            turn: Animated::new(0.0, role::page()).with_rest(0.002),
            usb: super::usb(),
            usb_for: 0.0,
            check: Check::Idle,
            kept: super::kept(),
            inputs: ListState::default(),
            results: ListState::default(),
            kept_list: ListState::default(),
            naming: None,
            forgetting: None,
            drawn: Snapshot::default(),
            pending: true,
            since_draw: 0.0,
            quiet_for: 0.0,
            rumble_for: 0.0,
        }
    }

    /// For the gallery: the page it is turned to, at once.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn on_page(mut self, page: usize) -> Self {
        self.page = page;
        self.turn.jump(page as f32);
        self
    }

    #[cfg(not(target_arch = "aarch64"))]
    pub fn pads_mut(&mut self) -> &mut Pads {
        &mut self.pads
    }

    /// For the gallery: a check run on the simulated controller, to its end.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn run_check(&mut self, until: f32) {
        self.start();
        let mut t = 0.0;
        while t < until && !matches!(self.check, Check::Done(_)) {
            self.pads.poll(1.0 / 60.0);
            self.advance(1.0 / 60.0);
            t += 1.0 / 60.0;
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    pub fn naming(mut self, label: &str) -> Self {
        self.naming = Some(Line::new("keep as", label, 24));
        self
    }

    #[cfg(test)]
    pub fn result(&self) -> Option<&Record> {
        match &self.check {
            Check::Done(r) => Some(r),
            _ => None,
        }
    }

    fn current(&self) -> Option<&Pad> {
        self.pads.list.get(self.pad)
    }

    fn checking(&self) -> bool {
        matches!(self.check, Check::Rest { .. } | Check::Sweep { .. })
    }

    fn start(&mut self) {
        let Some(pad) = self.pads.list.get_mut(self.pad) else { return };
        pad.reports.clear();
        self.check = Check::Rest { t: 0.0, samples: [Vec::new(), Vec::new()] };
        self.results = ListState::default();
    }

    fn cancel(&mut self) {
        self.check = Check::Idle;
        self.pads.sweep(false);
    }

    /// The check takes what the controller reported. True while it runs
    /// (its progress moves).
    fn advance(&mut self, dt: f32) -> bool {
        let Some(pad) = self.pads.list.get_mut(self.pad) else {
            let was = self.checking();
            if was {
                self.cancel();
            }
            return was;
        };
        let reports = std::mem::take(&mut pad.reports);
        // A still stick sends nothing (evdev passes only changes): where
        // nothing came, it is where it was.
        let now = [pad.stick(0), pad.stick(1)];
        let sticks: Vec<[(f32, f32); 2]> = if reports.is_empty() { vec![now] } else { reports.iter().map(|r| r.sticks).collect() };
        match &mut self.check {
            Check::Rest { t, samples } => {
                for s in &sticks {
                    samples[0].push(s[0]);
                    samples[1].push(s[1]);
                }
                *t += dt;
                if *t >= REST_S {
                    let rest = [health::rest(&samples[0]), health::rest(&samples[1])];
                    self.check = Check::Sweep { t: 0.0, rest, samples: [Vec::new(), Vec::new()], covered: [[false; health::SECTORS]; 2], times: Vec::new() };
                    self.pads.sweep(true);
                }
                true
            }
            Check::Sweep { t, samples, covered, times, .. } => {
                times.extend(reports.iter().map(|r| r.at_us));
                for s in &sticks {
                    for k in 0..2 {
                        samples[k].push(s[k]);
                        if let Some(at) = health::sector(s[k].0, s[k].1) {
                            covered[k][at] = true;
                        }
                    }
                }
                *t += dt;
                let all = covered.iter().all(|c| c.iter().all(|x| *x));
                if (all && *t >= SWEEP_MIN_S) || *t >= SWEEP_MAX_S {
                    self.finish();
                }
                true
            }
            _ => false,
        }
    }

    /// The sweep is over (or cut short with Enter): the result.
    fn finish(&mut self) {
        let Check::Sweep { rest, samples, times, .. } = &self.check else { return };
        let Some(pad) = self.pads.list.get(self.pad) else { return };
        let reach = [health::reach(&samples[0]), health::reach(&samples[1])];
        let mut record = Record::new(pad, *rest, reach, health::rate(times));
        // Checked before: it keeps the name the team gave it.
        if let Some((_, old)) = self.kept.iter().find(|(_, r)| r.id == record.id) {
            record.label = old.label.clone();
        }
        self.check = Check::Done(record);
        self.pads.sweep(false);
        self.results = ListState::default();
    }

    /// What the pages show of the controller now.
    fn snapshot(&self) -> Snapshot {
        let Some(pad) = self.current() else { return Snapshot { count: self.pads.list.len(), ..Snapshot::default() } };
        Snapshot {
            count: self.pads.list.len(),
            axes: pad.axes.iter().map(|a| (a.value, (a.info.max - a.info.min).max(1))).collect(),
            buttons: pad.buttons.iter().map(|(_, d)| *d).collect(),
            // The rate, as its figure reads.
            rate: pad.rate().map(|r| (r / 10.0).round() as i32),
        }
    }

    /// Whether anything shown has changed since it was drawn: an axis by a
    /// 96th of its range or more (half a pixel of a stick's well), so a
    /// stick's shake at rest draws nothing; a button; the rate's figure.
    fn moved(&self) -> bool {
        let (now, then) = (self.snapshot(), &self.drawn);
        now.count != then.count
            || now.buttons != then.buttons
            || now.rate != then.rate
            || now.axes.len() != then.axes.len()
            || now.axes.iter().zip(&then.axes).any(|((v, span), (w, _))| (*v as i64 - *w as i64).abs() * 96 >= *span as i64)
    }

    fn rumble(&mut self, say: &mut Option<String>) {
        if self.current().is_some_and(|p| p.rumble) && self.pads.rumble(self.pad, RUMBLE_MS) {
            self.rumble_for = RUMBLE_MS as f32 / 1000.0;
            self.pending = true;
        } else if self.current().is_some() {
            *say = Some("this controller has no rumble".into());
        }
    }

    // ------------------------------------------------------------ drawing ---

    fn draw_page(&mut self, g: &mut Gfx, page: usize) {
        if page == KEPT {
            return self.draw_kept(g);
        }
        if self.current().is_none() {
            return self.draw_unplugged(g);
        }
        match page {
            LIVE => self.draw_live(g),
            INPUTS => self.draw_inputs(g),
            _ => self.draw_check(g),
        }
    }

    fn draw_unplugged(&self, g: &mut Gfx) {
        if self.usb == Usb::Host {
            return g.empty("sports_esports", "Plug a controller into the USB-A port");
        }
        let t = g.tokens;
        let style = g.ty.body;
        let lines = crate::help::wrap(g, self.usb.advice(), style, W - 4.0 * PAD);
        let pitch = style.size + 5.0;
        let block = 30.0 + lines.len() as f32 * pitch;
        let top = g.top();
        let y0 = top + ((g.above_hint() - top - block) * 0.5).max(0.0);
        g.icon("usb", false, W * 0.5, y0 + 10.0, 22.0, t.amber_ink);
        for (k, line) in lines.iter().enumerate() {
            g.text_at(W * 0.5, y0 + 30.0 + pitch * (k as f32 + 0.5), Align::Center, line, style, t.ink);
        }
    }

    fn draw_live(&self, g: &mut Gfx) {
        let Some(pad) = self.current() else { return };
        let t = g.tokens;
        let ty = g.ty;
        let ox = g.ox;
        let (top, bottom) = (g.top(), g.above_hint());
        let cap = ty.caption;
        let sony = pad.sony();
        let num = pad.layout.numbered;
        let name = |code: u16| evdev::button_name(code, sony, num);
        // The shoulders: each trigger's bar, then its bumper.
        let sh = cap.size + 6.0;
        let word_w = g.measure(&name(BTN_TL2), cap).max(g.measure(&name(BTN_TR2), cap)) + 4.0;
        let bumper_w = g.measure(&name(BTN_TL), cap).max(g.measure(&name(BTN_TR), cap)) + 14.0;
        let meter_w = 58.0;
        let (lt, lb) = if num { (BTN_JOYSTICK + 6, BTN_JOYSTICK + 4) } else { (BTN_TL2, BTN_TL) };
        let (rt, rb) = if num { (BTN_JOYSTICK + 7, BTN_JOYSTICK + 5) } else { (BTN_TR2, BTN_TR) };
        let cy = top + sh * 0.5;
        g.text_at(PAD, cy, Align::Left, &name(lt), cap, t.dim);
        g.meter(Rect::new(PAD + word_w, cy - 3.5, meter_w, 7.0), pad.trigger(0), t.ice);
        chip(g, Rect::new(PAD + word_w + meter_w + 5.0, top, bumper_w, sh), &name(lb), pad.down(lb));
        g.text_at(W - PAD, cy, Align::Right, &name(rt), cap, t.dim);
        g.meter(Rect::new(W - PAD - word_w - meter_w, cy - 3.5, meter_w, 7.0), pad.trigger(1), t.ice);
        chip(g, Rect::new(W - PAD - word_w - meter_w - 5.0 - bumper_w, top, bumper_w, sh), &name(rb), pad.down(rb));
        // The body, as a controller is laid out: the left stick and the
        // face buttons high, the D-pad and the right stick low.
        let body = top + sh + 6.0;
        let r = ((bottom - body - 10.0) / 3.6).clamp(18.0, 26.0);
        let ya = body + r + 1.0;
        let yb = bottom - r - 8.0;
        let thumbs = if num { (BTN_JOYSTICK + 10, BTN_JOYSTICK + 11) } else { (BTN_THUMBL, BTN_THUMBR) };
        // The deadzone its last check suggested, ringed in each well.
        let dz = self.kept.iter().find(|(_, k)| k.id == pad.id()).map(|(_, k)| k.deadzone);
        stick(g, PAD + r + 2.0, ya, r, pad.stick(0), pad.down(thumbs.0), None, dz.map(|d| d[0]));
        stick(g, 220.0, yb, r, pad.stick(1), pad.down(thumbs.1), None, dz.map(|d| d[1]));
        let rb = (cap.size * 0.8).max(9.0);
        dpad(g, 98.0, yb, rb * 1.2, pad.dpad());
        face(g, pad, W - PAD - rb * 3.05 - 1.0, ya);
        // Between them: view, guide and menu; how often it reports.
        let (view, menu, guide) = if num { (BTN_JOYSTICK + 8, BTN_JOYSTICK + 9, 0) } else { (BTN_SELECT, BTN_START, BTN_MODE) };
        for (x, code, rad) in [(142.0, view, 5.0), (160.0, guide, 7.0), (178.0, menu, 5.0)] {
            if pad.has(code) {
                let on = pad.down(code);
                g.canvas.circle(x + ox, ya, rad, if on { t.ice } else { t.meter }, 1.0);
            }
        }
        // To ten a second: the gaps between reports wobble by microseconds.
        let rate = pad.rate().map(|r| format!("{} Hz", (r / 10.0).round() * 10.0)).unwrap_or_else(|| "still".into());
        g.text_at(160.0, ya + 16.0, Align::Center, &rate, cap, t.dim);
        if self.pads.list.len() > 1 {
            g.text_at(160.0, yb, Align::Center, &format!("{} of {}", self.pad + 1, self.pads.list.len()), cap, t.dim);
        }
        if self.rumble_for > 0.0 {
            chip(g, Rect::new(160.0 - 30.0, yb + r - sh + 4.0, 60.0, sh), "rumble", true);
        }
    }

    fn draw_inputs(&mut self, g: &mut Gfx) {
        let Some(pad) = self.pads.list.get(self.pad) else { return };
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let chip_rows = pad.buttons.len().div_ceil(CHIPS);
        let len = pad.axes.len() + chip_rows;
        let w = W - 2.0 * PAD - 5.0;
        let rows = self.inputs.layout(g, len, top, row_h, 3.0, bottom);
        let focus = self.inputs.focus;
        for (i, y) in rows {
            let r = Rect::new(PAD, y, w, row_h);
            if let Some(a) = pad.axes.get(i) {
                let value = format!("{}", a.value);
                g.row(r, &evdev::axis_name(a.code), &value, t.dim, i == focus);
                let value_w = g.measure(&format!("{}", a.info.min.min(-a.info.max.abs())), ty.label).max(g.measure(&value, ty.label));
                let bar = Rect::new(r.x + 72.0, r.cy() - 3.5, r.w - 72.0 - value_w - 22.0, 7.0);
                if a.info.min < 0 {
                    g.balance(bar, a.centred(), t.ice);
                } else {
                    g.meter(bar, a.pulled(), t.ice);
                }
            } else {
                let k = i - pad.axes.len();
                let cw = (w - (CHIPS as f32 - 1.0) * 4.0) / CHIPS as f32;
                for (j, (code, down)) in pad.buttons.iter().skip(k * CHIPS).take(CHIPS).enumerate() {
                    chip(g, Rect::new(PAD + j as f32 * (cw + 4.0), y, cw, row_h), &evdev::button_name(*code, pad.sony(), pad.layout.numbered), *down);
                }
                if i == focus {
                    g.focus_ring(r, row_h * 0.5);
                }
            }
        }
        self.inputs.scrollbar(g, len, top, bottom);
    }

    fn draw_check(&mut self, g: &mut Gfx) {
        if let Check::Done(record) = &self.check {
            let record = record.clone();
            return self.draw_results(g, &record);
        }
        let Some(pad) = self.current() else { return };
        let t = g.tokens;
        let ty = g.ty;
        let (top, bottom) = (g.top(), g.above_hint());
        let (say, progress, covered): (&str, f32, Option<[[bool; health::SECTORS]; 2]>) = match &self.check {
            Check::Rest { t, .. } => ("Hands off the sticks", t / REST_S, None),
            Check::Sweep { t, covered, .. } => ("Roll both sticks round their edges", (t / SWEEP_MAX_S).max(covered.iter().flatten().filter(|c| **c).count() as f32 / (2 * health::SECTORS) as f32), Some(*covered)),
            _ => ("Check its sticks, buttons and reports", 0.0, None),
        };
        let name = ty.name;
        let line = crate::sheet::first_fitting(g, &[say, "Roll the sticks round", "Check it"], name, W - 2.0 * PAD);
        g.text_at(W * 0.5, top + name.size * 0.5, Align::Center, line, name, t.ink);
        let bar_y = top + name.size + 6.0;
        if self.checking() {
            g.meter(Rect::new(PAD + 40.0, bar_y, W - 2.0 * PAD - 80.0, 6.0), progress, t.signal);
        } else {
            let last = self.kept.iter().find(|(_, r)| r.id == pad.id()).map(|(_, r)| format!("last kept: {} · deadzone {:.2}", r.verdict.word(), r.deadzone[0].max(r.deadzone[1])));
            let words = last.unwrap_or_else(|| "2 s hands off, then a few turns of each stick".into());
            let fitted = g.text.fit(&words, &ty.caption, W - 2.0 * PAD);
            g.text_at(W * 0.5, bar_y + 3.0, Align::Center, &fitted, ty.caption, t.dim);
        }
        let y0 = bar_y + 12.0;
        let r = ((bottom - y0 - 10.0) * 0.5).clamp(16.0, 34.0);
        let cy = y0 + 4.0 + r;
        for (k, cx) in [W * 0.3, W * 0.7].into_iter().enumerate() {
            let sectors = covered.map(|c| c[k]);
            stick(g, cx, cy, r, pad.stick(k), false, sectors.as_ref(), None);
            let word = if k == 0 { "left" } else { "right" };
            g.text_at(cx + r + 10.0, cy - r + ty.caption.size * 0.5, Align::Left, word, ty.caption, t.dim);
        }
    }

    fn draw_results(&mut self, g: &mut Gfx, rec: &Record) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let rows = result_rows(rec);
        let w = W - 2.0 * PAD - 5.0;
        for (i, y) in self.results.layout(g, rows.len(), top, row_h, 3.0, bottom) {
            let (name, value) = &rows[i];
            let color = if i == 0 { verdict_color(g, rec.verdict) } else { t.dim };
            g.row(Rect::new(PAD, y, w, row_h), name, value, color, i == self.results.focus);
        }
        self.results.scrollbar(g, rows.len(), top, bottom);
    }

    fn draw_kept(&mut self, g: &mut Gfx) {
        if self.kept.is_empty() {
            return g.empty("sports_esports", "No checks kept yet: check a controller, then press enter to keep it");
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let w = W - 2.0 * PAD - 5.0;
        let len = self.kept.len();
        for (i, y) in self.kept_list.layout(g, len, top, row_h, 3.0, bottom) {
            let (_, r) = &self.kept[i];
            let value = format!("{} · {:.2}", r.verdict.word(), r.deadzone[0].max(r.deadzone[1]));
            let color = verdict_color(g, r.verdict);
            let title = r.title().to_owned();
            g.row(Rect::new(PAD, y, w, row_h), &title, &value, color, i == self.kept_list.focus);
        }
        self.kept_list.scrollbar(g, len, top, bottom);
    }

    fn hint(&self, g: &mut Gfx) {
        if self.naming.is_some() {
            return g.hint_any(&["enter: keep it · esc: not now", "enter: keep it"]);
        }
        if self.forgetting.is_some() {
            return g.hint_any(&["backspace again: forget it · esc: keep it", "backspace again: forget it"]);
        }
        let name = self.current().map(|p| short_name(&p.name));
        match self.page {
            KEPT if !self.kept.is_empty() => {
                let r = &self.kept[self.kept_list.focus.min(self.kept.len() - 1)].1;
                let when = chrono::TimeZone::timestamp_opt(&chrono::Local, r.when, 0).single().map(|d| d.format("%b %-d").to_string()).unwrap_or_default();
                let about = format!("{} · {when}", r.model);
                g.hint_any(&[&format!("best first · {about}"), &about, "best first"]);
            }
            KEPT => g.hint_any(&["←: the check", "←: back"]),
            _ if name.is_none() => g.hint_any(&["→: the checks kept", "→: kept"]),
            CHECK => match &self.check {
                Check::Done(_) => g.hint_any(&["enter: keep it · r: check again · ←→: pages", "enter: keep · r: again", "enter: keep"]),
                Check::Sweep { .. } => g.hint_any(&["enter: done now · esc: stop", "enter: done"]),
                Check::Rest { .. } => g.hint_any(&["esc: stop the check", "esc: stop"]),
                Check::Idle => g.hint_any(&["enter: check this controller · ←→: pages", "enter: check it"]),
            },
            _ => {
                let name = name.unwrap_or_default();
                let rumble = if self.current().is_some_and(|p| p.rumble) { " · r: rumble" } else { "" };
                g.hint_any(&[&format!("{name}{rumble} · ←→: pages"), &format!("{name}{rumble}"), &name]);
            }
        }
    }
}

impl Screen for Gamepad {
    fn title(&self) -> &str {
        match self.page {
            INPUTS => "Every input",
            CHECK => "Check",
            KEPT => "Kept checks",
            _ => "Gamepad",
        }
    }

    fn app(&self) -> &str {
        "Gamepad"
    }

    fn wants_text(&self) -> bool {
        self.naming.is_some()
    }

    fn keeps_awake(&self) -> bool {
        self.checking() || (self.current().is_some() && self.quiet_for < 60.0)
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.turn.tick(dt)
    }

    fn ambient_tick(&mut self, dt: f32, _robot: &Robot) -> bool {
        self.since_draw += dt;
        self.quiet_for += dt;
        let count = self.pads.list.len();
        if self.pads.poll(dt) {
            self.quiet_for = 0.0;
        }
        if self.pads.list.len() != count {
            self.pad = self.pad.min(self.pads.list.len().saturating_sub(1));
            self.pending = true;
        }
        if self.advance(dt) {
            self.pending = true;
        }
        if self.rumble_for > 0.0 {
            self.rumble_for -= dt;
            if self.rumble_for <= 0.0 {
                self.pending = true;
            }
        }
        if self.pads.list.is_empty() {
            self.usb_for += dt;
            if self.usb_for >= 2.0 {
                self.usb_for = 0.0;
                let now = super::usb();
                self.pending |= now != self.usb;
                self.usb = now;
            }
        }
        if self.moved() {
            self.pending = true;
        }
        self.pending && self.since_draw >= FRAME
    }

    /// A change waiting for its frame, the check's progress, the rumble's
    /// word: looked in on as they need. Otherwise the loop's own look (and
    /// on the handheld a report wakes it anyway: the shell reads every
    /// input node, the controller's too).
    fn ambient(&self) -> Option<std::time::Duration> {
        let secs = if self.pending {
            Some((FRAME - self.since_draw).max(0.002))
        } else if self.checking() {
            Some(FRAME)
        } else if self.rumble_for > 0.0 {
            Some(self.rumble_for)
        } else {
            None
        };
        secs.map(std::time::Duration::from_secs_f32)
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(line) = &mut self.naming {
            match line.key(key, |c| c.is_alphanumeric() || " -_'#".contains(c)) {
                Typed::More => {}
                Typed::Cancelled => self.naming = None,
                Typed::Done(label) => {
                    self.naming = None;
                    if let Check::Done(record) = &mut self.check {
                        record.label = label;
                        *cx.say = Some(match super::keep(record) {
                            Ok(_) => format!("{} kept", record.title()),
                            Err(e) => format!("could not keep it: {e}"),
                        });
                        self.kept = super::kept();
                    }
                }
            }
            return Nav::Stay;
        }
        if key != Key::Backspace && key != Key::Delete {
            self.forgetting = None;
        }
        match key {
            Key::Back if self.checking() => self.cancel(),
            Key::Back | Key::Leave => {
                self.pads.sweep(false);
                return Nav::Pop;
            }
            Key::Right | Key::Tab if self.current().is_none() && self.page != KEPT => self.page = KEPT,
            Key::Left if self.current().is_none() && self.page == KEPT => self.page = LIVE,
            Key::Right | Key::Tab if self.page + 1 < PAGES => self.page += 1,
            Key::Tab => self.page = 0,
            Key::Left if self.page > 0 => self.page -= 1,
            Key::Char('r') if self.page == CHECK && matches!(self.check, Check::Done(_)) => self.start(),
            Key::Char('r') if self.page != KEPT => self.rumble(cx.say),
            Key::Enter => match self.page {
                LIVE | INPUTS if self.current().is_some() => {
                    self.page = CHECK;
                    if !self.checking() {
                        self.start();
                    }
                }
                CHECK => match &self.check {
                    Check::Idle if self.current().is_some() => self.start(),
                    Check::Sweep { .. } => self.finish(),
                    Check::Done(r) => {
                        let label = if r.label.is_empty() { String::new() } else { r.label.clone() };
                        self.naming = Some(Line::new("keep as", &label, 24));
                    }
                    _ => {}
                },
                _ => {}
            },
            Key::Backspace | Key::Delete if self.page == KEPT && !self.kept.is_empty() => {
                let at = self.kept_list.focus.min(self.kept.len() - 1);
                if self.forgetting == Some(at) {
                    let (path, r) = &self.kept[at];
                    let gone = std::fs::remove_file(path).is_ok();
                    *cx.say = Some(if gone { format!("{} forgotten", r.title()) } else { "could not forget it".into() });
                    self.forgetting = None;
                    self.kept = super::kept();
                } else {
                    self.forgetting = Some(at);
                }
            }
            other => match self.page {
                LIVE if matches!(other, Key::Up | Key::Down) && self.pads.list.len() > 1 => {
                    let n = self.pads.list.len();
                    self.pad = if other == Key::Down { (self.pad + 1) % n } else { (self.pad + n - 1) % n };
                }
                INPUTS => {
                    let len = self.current().map_or(0, |p| p.axes.len() + p.buttons.len().div_ceil(CHIPS));
                    crate::screens::list_key(&mut self.inputs, other, len);
                }
                CHECK => {
                    if let Check::Done(r) = &self.check {
                        crate::screens::list_key(&mut self.results, other, result_rows(r).len());
                    }
                }
                KEPT => crate::screens::list_key(&mut self.kept_list, other, self.kept.len()),
                _ => {}
            },
        }
        if self.turn.target() != self.page as f32 {
            self.turn.set(self.page as f32);
        }
        self.pending = true;
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        if g.calm {
            self.turn.jump(self.page as f32);
        }
        let base = g.ox;
        let at = self.turn.value();
        for page in 0..PAGES {
            let ox = (page as f32 - at) * W;
            if ox.abs() >= W {
                continue;
            }
            g.ox = base + ox.round();
            self.draw_page(g, page);
        }
        g.ox = base;
        if let Some(line) = &self.naming {
            line.draw_at(g, true);
        }
        self.hint(g);
        self.drawn = self.snapshot();
        self.pending = false;
        self.since_draw = 0.0;
    }
}

/// A controller's name without its maker's long company name.
fn short_name(name: &str) -> String {
    let mut s = name.trim();
    for maker in ["Sony Interactive Entertainment ", "Sony Computer Entertainment ", "Microsoft "] {
        s = s.strip_prefix(maker).unwrap_or(s);
    }
    s.to_owned()
}

/// The result's rows: the verdict, then each measure, left · right.
fn result_rows(r: &Record) -> Vec<(String, String)> {
    let both = |f: &dyn Fn(usize) -> String| format!("L {} · R {}", f(0), f(1));
    let throw = |k: usize| {
        let least = r.reach[k].iter().fold(1.0f32, |a, b| a.min(*b));
        if health::full(&r.reach[k]) {
            "full".to_owned()
        } else {
            format!("{:.0}%", least * 100.0)
        }
    };
    let advice = match r.verdict {
        health::Verdict::Good => "good · bring it",
        health::Verdict::Fair => "fair · a spare",
        health::Verdict::Worn => "worn · leave it",
    };
    let mut rows = vec![
        ("verdict".to_owned(), advice.to_owned()),
        ("deadzone".to_owned(), both(&|k| format!("{:.2}", r.deadzone[k]))),
        ("drift".to_owned(), both(&|k| format!("{:.3}", r.rest[k].drift_len()))),
        ("shake".to_owned(), both(&|k| format!("{:.3}", r.rest[k].noise))),
        ("throw".to_owned(), both(&throw)),
    ];
    rows.push(("reports".into(), r.rate.map(|x| format!("{:.0} Hz · worst {:.0} ms", x.hz, x.worst_ms)).unwrap_or_else(|| "none while moving".into())));
    rows.push(("controller".into(), r.model.clone()));
    rows.push(("id".into(), r.id.clone()));
    rows
}

fn verdict_color(g: &Gfx, v: health::Verdict) -> Color {
    match v {
        health::Verdict::Good => g.good(),
        health::Verdict::Fair => g.tokens.amber_ink,
        health::Verdict::Worn => g.fault(),
    }
}

/// A capsule with a word: ice when on.
fn chip(g: &mut Gfx, r: Rect, word: &str, on: bool) {
    let t = g.tokens;
    g.surface(r, r.h * 0.5, if on { t.ice } else { t.meter });
    let cap = g.ty.caption;
    let fitted = g.text.fit(word, &cap, r.w - 6.0);
    g.text_at(r.cx(), r.cy(), Align::Center, &fitted, cap, if on { t.on_ice } else { t.ink });
}

/// A stick: its well, where it is (−1 to 1 each way), and a bar for each
/// axis along its foot and side. Pressed in, the well fills with ice. A
/// check's sweep rings it with the sectors reached.
#[allow(clippy::too_many_arguments)]
fn stick(g: &mut Gfx, cx: f32, cy: f32, r: f32, at: (f32, f32), pressed: bool, sectors: Option<&[bool; health::SECTORS]>, deadzone: Option<f32>) {
    let t = g.tokens;
    let ox = g.ox;
    let (x, y) = (cx + ox, cy);
    g.canvas.circle(x, y, r, if pressed { t.ice } else { t.meter }, 1.0);
    let hair = if pressed { t.on_ice } else { t.faint };
    g.canvas.fill_rect(Rect::new(x - r + 4.0, y - 0.5, 2.0 * r - 8.0, 1.0), hair, 0.7);
    g.canvas.fill_rect(Rect::new(x - 0.5, y - r + 4.0, 1.0, 2.0 * r - 8.0), hair, 0.7);
    if let Some(dz) = deadzone {
        g.canvas.arc(x, y, ((r - 5.0) * dz).max(2.0), 1.0, 0.0, TAU, t.dim, 0.8);
    }
    if let Some(sectors) = sectors {
        let n = sectors.len() as f32;
        for (k, on) in sectors.iter().enumerate() {
            let start = k as f32 / n * TAU + 0.04;
            g.canvas.arc(x, y, r + 4.0, 2.5, start, TAU / n - 0.08, if *on { t.ice } else { t.faint }, 1.0);
        }
    }
    // The dot, inside the well, ringed in the well's colour.
    let (dx, dy) = (at.0.clamp(-1.0, 1.0), at.1.clamp(-1.0, 1.0));
    let (px, py) = (x + dx * (r - 5.0), y + dy * (r - 5.0));
    g.canvas.circle(px, py, 6.5, if pressed { t.ice } else { t.meter }, 1.0);
    g.canvas.circle(px, py, 4.5, if pressed { t.on_ice } else { t.ink }, 1.0);
    if sectors.is_none() {
        // Each axis as a bar: x along the foot, y up the side.
        g.balance(Rect::new(cx - r, cy + r + 3.0, 2.0 * r, 3.0), dx, t.ice);
        vbalance(g, Rect::new(cx + r + 3.0, cy - r, 3.0, 2.0 * r), dy);
    }
}

/// A balance standing up: down positive, as evdev reads.
fn vbalance(g: &mut Gfx, r: Rect, value: f32) {
    let t = g.tokens;
    let track = r.offset(g.ox, 0.0);
    let radius = r.w * 0.5;
    g.canvas.rrect(track, radius, t.meter, 1.0);
    let mid = track.y + track.h * 0.5;
    let reach = track.h * 0.5 * value.clamp(-1.0, 1.0);
    if reach.abs() > 0.5 {
        let (y, h) = if reach > 0.0 { (mid, reach) } else { (mid + reach, -reach) };
        g.canvas.rrect_clipped(Rect::new(track.x, y, track.w, h), 1.0, track, radius, t.ice, 1.0);
    }
    g.canvas.rrect(Rect::new(track.x - 1.5, mid - 0.5, track.w + 3.0, 1.0), 0.5, t.dim, 1.0);
}

/// The D-pad: four arms round a hub, the pressed ones in ice.
fn dpad(g: &mut Gfx, cx: f32, cy: f32, arm: f32, at: (i32, i32)) {
    let t = g.tokens;
    let w = arm;
    let half = w * 0.5;
    for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
        let on = (dx != 0 && at.0 == dx) || (dy != 0 && at.1 == dy);
        let r = if dx == 0 { Rect::new(cx - half, if dy < 0 { cy - half - arm } else { cy + half }, w, arm) } else { Rect::new(if dx < 0 { cx - half - arm } else { cx + half }, cy - half, arm, w) };
        g.surface(r, 3.0, if on { t.ice } else { t.meter });
    }
    g.surface(Rect::new(cx - half, cy - half, w, w), 1.0, t.meter);
}

/// The four face buttons in their diamond, named as the controller names
/// them: A B X Y, the PlayStation's shapes, or numbers.
fn face(g: &mut Gfx, pad: &Pad, cx: f32, cy: f32) {
    let t = g.tokens;
    let ox = g.ox;
    let cap = g.ty.caption;
    let rb = (cap.size * 0.8).max(9.0);
    let d = rb * 2.05;
    // (dx, dy, code): where each sits. The Xbox driver gives X the code
    // the PlayStation drivers give the triangle (see evdev::BTN_NORTH).
    let places: [(f32, f32, u16); 4] = if pad.layout.numbered {
        [(-1.0, 0.0, BTN_JOYSTICK), (0.0, 1.0, BTN_JOYSTICK + 1), (1.0, 0.0, BTN_JOYSTICK + 2), (0.0, -1.0, BTN_JOYSTICK + 3)]
    } else if pad.sony() {
        [(0.0, 1.0, BTN_SOUTH), (1.0, 0.0, BTN_EAST), (0.0, -1.0, BTN_NORTH), (-1.0, 0.0, BTN_WEST)]
    } else {
        [(0.0, 1.0, BTN_SOUTH), (1.0, 0.0, BTN_EAST), (-1.0, 0.0, BTN_NORTH), (0.0, -1.0, BTN_WEST)]
    };
    for (dx, dy, code) in places {
        let (x, y) = (cx + dx * d, cy + dy * d);
        let on = pad.down(code);
        g.canvas.circle(x + ox, y, rb, if on { t.ice } else { t.meter }, 1.0);
        let ink = if on { t.on_ice } else { t.ink };
        if pad.sony() && !pad.layout.numbered {
            shape(g, x + ox, y, rb * 0.5, code, ink);
        } else {
            let word = evdev::button_name(code, false, pad.layout.numbered);
            g.text_at(x, y, Align::Center, &word, cap, ink);
        }
    }
}

/// A PlayStation button's shape, drawn (the font is Latin only).
fn shape(g: &mut Gfx, x: f32, y: f32, s: f32, code: u16, ink: Color) {
    match code {
        BTN_SOUTH => {
            g.canvas.polyline(&[(x - s, y - s), (x + s, y + s)], 1.5, ink, 1.0);
            g.canvas.polyline(&[(x - s, y + s), (x + s, y - s)], 1.5, ink, 1.0);
        }
        BTN_EAST => g.canvas.arc(x, y, s, 1.5, 0.0, TAU, ink, 1.0),
        BTN_NORTH => g.canvas.polyline(&[(x, y - s), (x + s, y + s * 0.8), (x - s, y + s * 0.8), (x, y - s)], 1.5, ink, 1.0),
        _ => g.canvas.stroke_rrect(Rect::new(x - s, y - s, 2.0 * s, 2.0 * s), 1.0, 1.5, ink, 1.0),
    }
}
