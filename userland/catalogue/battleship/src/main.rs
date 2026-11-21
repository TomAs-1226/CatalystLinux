//! Battleship for Catalyst: your fleet against the handheld.
//!
//! A native rebuild of "Battleship" by yutacar (MIT), from the CardputerZero
//! app store: ten by ten, five ships a side (carrier 5, battleship 4,
//! cruiser 3, submarine 3, destroyer 2) that may touch but not overlap, and
//! shots that always alternate. The computer player hunts by probability
//! density, at three levels (see `ai`). The original's two-player game over
//! Bluetooth or Wi-Fi is left out: here it is you against the handheld.
//!
//! The first page chooses the level and shows the record at each; the game
//! itself is one page on the whole panel (`battle`).

mod ai;
mod battle;
mod game;
mod record;
mod sea;

use catalyst_kit::prelude::*;

use crate::ai::Level;
use crate::battle::Battle;
use crate::record::Record;

/// The first page: the three levels, each with its record.
struct Start {
    list: List,
    record: Record,
    /// Each level's record in words, made when it is read.
    words: [String; 3],
    /// The line about the level in focus, fading in when it changes.
    about: Animated,
}

impl Start {
    fn new() -> Start {
        let record = Record::open();
        let mut list = List::default();
        list.focus = record.last_level().index();
        let words = Level::ALL.map(|l| record.words(l));
        Start { list, record, words, about: Animated::new(1.0, role::effect()).with_rest(0.01) }
    }

    fn level(&self) -> Level {
        Level::ALL[self.list.focus.min(Level::ALL.len() - 1)]
    }
}

impl Page for Start {
    fn title(&self) -> &str {
        "Battleship"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        match key {
            Key::Back => Flow::Quit,
            Key::Enter => {
                let level = self.level();
                // Remembered for next time; a failure only forgets it.
                let _ = self.record.set_last_level(level);
                Flow::Push(Box::new(Battle::new(level, cx.calm())))
            }
            other => {
                let before = self.list.focus;
                self.list.key(other, Level::ALL.len());
                if self.list.focus != before && !cx.calm() {
                    self.about.jump(0.0);
                    self.about.set(1.0);
                }
                Flow::Stay
            }
        }
    }

    /// Back from a game: its result is in the record.
    fn resume(&mut self, _cx: &mut Cx) {
        self.record = Record::open();
        self.words = Level::ALL.map(|l| self.record.words(l));
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.about.tick(dt)
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (t, ty) = (ui.tokens, ui.ty);
        let (top, h, bottom) = (ui.top(), ui.row_h(), ui.bottom());
        let gap = 3.0;
        let count = Level::ALL.len();
        for (i, y) in self.list.layout(ui, count, top, h, gap, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD, h);
            ui.row(r, Level::ALL[i].name(), &self.words[i], t.dim, i == self.list.focus);
        }
        self.list.end(ui, count, top, bottom);
        // How the level in focus plays, under the rows: shown whole or not at all.
        let style = ty.caption;
        let y = top + count as f32 * (h + gap) + 3.0;
        let pitch = (style.size * 1.32).round();
        let lines = ui.wrap(self.level().about(), style, W - 2.0 * PAD - 4.0);
        if y + pitch * lines.len() as f32 <= bottom + 2.0 {
            let alpha = self.about.value().clamp(0.0, 1.0);
            for (i, line) in lines.iter().enumerate() {
                ui.text_alpha(PAD + 2.0, y + pitch * (i as f32 + 0.5), line, style, t.dim, alpha);
            }
        }
        ui.hint_any(&["enter: play · ↑↓: level · esc: leave", "enter: play · esc: leave", "enter · esc"]);
    }
}

/// The gallery's own record, so every gallery draws the same start page
/// (and never touches the record of games really played).
fn gallery_record() {
    let dir = std::env::temp_dir().join("catalyst-battleship-gallery");
    let _ = std::fs::remove_dir_all(&dir);
    std::env::set_var("CATALYST_APP_DATA", &dir);
    let mut saved = Saved::open("record");
    for (key, value) in [("level", "normal"), ("easy.won", "6"), ("easy.lost", "1"), ("normal.won", "3"), ("normal.lost", "2")] {
        saved.set(key, value);
    }
    if let Err(why) = saved.save() {
        eprintln!("the gallery's record: {why}");
    }
}

fn main() {
    if std::env::args().any(|a| a == "--shots") {
        gallery_record();
    }
    // The gallery's deal (CATALYST_SEED 5805) hides the handheld's fleet so:
    // carrier down from (6, 5), battleship across from (2, 0), cruiser down
    // from (3, 4), submarine across from (3, 9), destroyer down from (9, 8).
    // The aim starts at (4, 4).
    use Key::*;
    catalyst_kit::launch(
        Start::new,
        &[
            ("1-start", &[]),
            // The carrier put down; the battleship in hand, turned.
            ("2-placing", &[Enter, Down, Down, Right, Enter, Down, Down, Right, Right, Right, Char('r')]),
            ("3-placed", &[Char('a')]),
            // A miss, the cruiser found and sunk, two hits on the carrier, misses.
            (
                "4-battle",
                &[Enter, Enter, Left, Enter, Down, Enter, Down, Enter, Right, Right, Right, Enter, Up, Enter, Up, Enter, Up, Up, Enter, Left, Left, Left, Left, Enter],
            ),
            // A hit on the battleship, and the handheld's answer.
            ("5-reply", &[Up, Up, Enter]),
            // Every ship found.
            (
                "6-end",
                &[
                    Right, Enter, Right, Enter, Right, Enter, Right, Down, Down, Down, Down, Down, Down, Down, Enter, Down, Enter, Down, Enter, Left, Enter, Left, Enter, Left, Enter, Right, Right,
                    Right, Right, Right, Right, Enter, Up, Enter,
                ],
            ),
        ],
    );
}
