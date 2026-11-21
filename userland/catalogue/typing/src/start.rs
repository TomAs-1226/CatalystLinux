//! The first page: the three rounds as rows, each with your best for the
//! word list chosen below them; the lists as mode buttons whose capsule
//! glides from one to the next.

use std::path::{Path, PathBuf};

use catalyst_kit::prelude::*;

use crate::model::Round;
use crate::play::Play;
use crate::records::Records;
use crate::words::Pack;

/// The word list's row, after the three rounds.
const LIST_ROW: usize = Round::ALL.len();

/// A round's seed for its words, from the app's random numbers.
pub fn seed_from(rng: &mut Rng) -> u32 {
    (rng.next() >> 32) as u32
}

pub struct Start {
    dir: PathBuf,
    records: Records,
    /// Best words a minute, by round and list: read when the page opens or
    /// comes back, never while drawing.
    bests: [[u32; 3]; 3],
    /// "best 54 wpm" for each round, in the list chosen.
    values: [String; 3],
    rng: Rng,
    focus: usize,
    pack: Pack,
    /// The selected list's capsule, by index, on a spring.
    pill: Animated,
    calm: bool,
}

impl Start {
    /// In the app's own folder, dealing from the clock (or `CATALYST_SEED`).
    pub fn new() -> Start {
        Start::at(&catalyst_kit::data_dir(), Rng::seeded())
    }

    pub fn at(dir: &Path, rng: Rng) -> Start {
        let records = Records::in_dir(dir);
        let (round, pack) = records.choice();
        let mut start = Start {
            dir: dir.to_owned(),
            records,
            bests: [[0; 3]; 3],
            values: Default::default(),
            rng,
            focus: round.index(),
            pack,
            pill: Animated::new(pack.index() as f32, role::release()).with_rest(0.002),
            calm: false,
        };
        start.read_bests();
        start
    }

    fn read_bests(&mut self) {
        for round in Round::ALL {
            for pack in Pack::ALL {
                self.bests[round.index()][pack.index()] = self.records.best(round, pack);
            }
        }
        self.write_values();
    }

    fn write_values(&mut self) {
        use std::fmt::Write;
        for (i, value) in self.values.iter_mut().enumerate() {
            value.clear();
            match self.bests[i][self.pack.index()] {
                0 => value.push('–'),
                best => {
                    let _ = write!(value, "best {best} wpm");
                }
            }
        }
    }

    fn choose_pack(&mut self, pack: Pack) {
        if pack == self.pack {
            return;
        }
        self.pack = pack;
        let to = pack.index() as f32;
        if self.calm {
            self.pill.jump(to);
        } else {
            self.pill.set(to);
        }
        self.write_values();
    }

    fn step_pack(&mut self, by: isize, wrap: bool) {
        let n = Pack::ALL.len() as isize;
        let at = self.pack.index() as isize + by;
        let at = if wrap { at.rem_euclid(n) } else { at.clamp(0, n - 1) };
        self.choose_pack(Pack::ALL[at as usize]);
    }
}

impl Page for Start {
    fn title(&self) -> &str {
        "Typing"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        self.calm = cx.calm();
        match key {
            Key::Back => return Flow::Quit,
            Key::Up => self.focus = self.focus.saturating_sub(1),
            Key::Down => self.focus = (self.focus + 1).min(LIST_ROW),
            Key::Home => self.focus = 0,
            Key::End => self.focus = LIST_ROW,
            Key::Left => self.step_pack(-1, false),
            Key::Right => self.step_pack(1, false),
            Key::Enter if self.focus == LIST_ROW => self.step_pack(1, true),
            Key::Enter => {
                let round = Round::ALL[self.focus];
                self.records.choose(round, self.pack);
                let seed = seed_from(&mut self.rng);
                return Flow::Push(Box::new(Play::new(round, self.pack, seed, Records::in_dir(&self.dir))));
            }
            _ => {}
        }
        Flow::Stay
    }

    /// Back from a round: its best may be new.
    fn resume(&mut self, _cx: &mut Cx) {
        self.records = Records::in_dir(&self.dir);
        self.read_bests();
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.pill.tick(dt)
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (t, ty) = (ui.tokens, ui.ty);
        self.calm = ui.calm();
        let (top, h, bottom) = (ui.top(), ui.row_h(), ui.bottom());
        let gap = 3.0;
        for (i, round) in Round::ALL.into_iter().enumerate() {
            let r = Rect::new(PAD, top + i as f32 * (h + gap), W - 2.0 * PAD, h);
            ui.row(r, round.name(), &self.values[i], t.dim, self.focus == i);
        }

        // The word lists, as mode buttons: a capsule of ice under the one chosen.
        let focused = self.focus == LIST_ROW;
        let press = if focused { ui.press() } else { 0.0 };
        let place = Rect::new(PAD, bottom - h, W - 2.0 * PAD, h);
        let track = place.inset(1.5 * press);
        let radius = track.h * 0.5;
        ui.surface(track, radius, if focused { t.surface2 } else { t.surface1 });
        let inner = track.inset(3.0);
        let seg = inner.w / Pack::ALL.len() as f32;
        let at = self.pill.value();
        ui.surface(Rect::new(inner.x + at * seg, inner.y, seg, inner.h), inner.h * 0.5, t.ice);
        let icon = (inner.h - 4.0).clamp(11.0, 15.0);
        for pack in Pack::ALL {
            let i = pack.index() as f32;
            let on = (1.0 - (at - i).abs()).clamp(0.0, 1.0);
            let ink = t.dim.mix(t.on_ice, on);
            let label = pack.key();
            let w = icon + 4.0 + ui.measure(label, ty.label);
            let x = inner.x + (i + 0.5) * seg - w * 0.5;
            ui.icon(pack.icon(), on > 0.5, x + icon * 0.5, inner.cy(), icon, ink);
            ui.text_at(x + icon + 4.0, inner.cy(), Align::Left, label, ty.label, ink);
        }
        if focused {
            ui.focus_ring(place, place.h * 0.5);
        }
        ui.hint_any(&["enter: start · ←→: word list · esc: leave", "enter: start · ←→: words", "enter · ←→ · esc"]);
    }
}
