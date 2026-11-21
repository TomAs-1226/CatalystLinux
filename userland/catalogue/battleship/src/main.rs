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
    /// That line wrapped to the page, and the level and type size it was
    /// wrapped for.
    about_lines: Vec<String>,
    wrapped_for: Option<(usize, u32)>,
}

impl Start {
    fn new() -> Start {
        let record = Record::open();
        let mut list = List::default();
        list.focus = record.last_level().index();
        let words = Level::ALL.map(|l| record.words(l));
        Start { list, record, words, about: Animated::new(1.0, role::effect()).with_rest(0.01), about_lines: Vec::new(), wrapped_for: None }
    }

    fn level(&self) -> Level {
        Level::ALL[self.list.focus.min(Level::ALL.len() - 1)]
    }
}

impl Page for Start {
    /// Not the app's name: the island already carries that, and beside it
    /// "Battleship" is cut short at the large text size. (The end of a game
    /// offers "esc: levels", back to here.)
    fn title(&self) -> &str {
        "Levels"
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
        let wrap_key = (self.list.focus, style.size.to_bits());
        if self.wrapped_for != Some(wrap_key) {
            self.about_lines = ui.wrap(self.level().about(), style, W - 2.0 * PAD - 4.0);
            self.wrapped_for = Some(wrap_key);
        }
        if y + pitch * self.about_lines.len() as f32 <= bottom + 2.0 {
            let alpha = self.about.value().clamp(0.0, 1.0);
            for (i, line) in self.about_lines.iter().enumerate() {
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
    // The island names the app, as it does when the shell starts it.
    if std::env::var_os("CATALYST_APP_NAME").is_none() {
        std::env::set_var("CATALYST_APP_NAME", "Battleship");
    }
    if std::env::args().any(|a| a == "--shots") {
        gallery_record();
    }
    // The gallery's deal (CATALYST_SEED 5805) hides the handheld's fleet so:
    // carrier down from (6, 5), battleship across from (2, 0), cruiser down
    // from (3, 4), submarine across from (3, 9), destroyer down from (9, 8).
    // The aim starts at (4, 4). The handheld's 17th shot finds your carrier.
    use Key::*;
    catalyst_kit::launch(
        Start::new,
        &[
            ("1-start", &[]),
            // The carrier put down; the battleship in hand, turned.
            ("2-placing", &[Enter, Down, Down, Right, Enter, Down, Down, Right, Right, Right, Char('r')]),
            ("3-placed", &[Char('a')]),
            // Fourteen shots: the cruiser found and sunk, two hits on the
            // carrier, misses about the board.
            (
                "4-battle",
                &[
                    Enter, Enter, Left, Enter, Down, Enter, Down, Enter, Right, Right, Right, Enter, Up, Enter, Up, Enter, Up, Up, Enter, Left, Left, Left, Left, Enter, Down, Down,
                    Down, Down, Down, Enter, Right, Right, Right, Right, Right, Right, Enter, Up, Up, Up, Enter, Up, Up, Enter, Up, Up, Enter,
                ],
            ),
            // Three hits on the battleship; the handheld's answer hits your carrier.
            ("5-reply", &[Left, Left, Left, Left, Left, Left, Enter, Right, Enter, Right, Enter]),
            // Every ship found.
            (
                "6-end",
                &[
                    Right, Enter, Right, Down, Down, Down, Down, Down, Down, Down, Enter, Down, Enter, Down, Enter, Left, Enter, Left, Enter, Left, Enter, Right, Right,
                    Right, Right, Right, Right, Enter, Up, Enter,
                ],
            ),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalyst_kit::{Look, Runner};

    fn frames(app: &mut Runner, n: usize) {
        for _ in 0..n {
            if app.tick(1.0 / 60.0) {
                app.draw();
            }
        }
    }

    #[test]
    fn a_whole_game_plays_to_the_end_by_keys() {
        let dir = std::env::temp_dir().join(format!("battleship-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("CATALYST_APP_DATA", &dir);
        let mut app = Runner::new(Box::new(Start::new()), Look::default());
        frames(&mut app, 60);
        assert_eq!(app.top_title(), "Levels");
        // Normal is in focus at first; Enter starts placing.
        app.key(Key::Enter);
        frames(&mut app, 60);
        assert_eq!(app.top_title(), "Battleship");
        // The carrier turned and put down, taken back with Backspace, put
        // down again; then the rest at random.
        app.key(Key::Char('r'));
        app.key(Key::Enter);
        app.key(Key::Backspace);
        frames(&mut app, 30);
        assert_eq!(app.top_title(), "Battleship", "Backspace takes the carrier back; it does not leave");
        app.key(Key::Enter);
        app.key(Key::Char('a'));
        frames(&mut app, 60);
        app.key(Key::Enter);
        frames(&mut app, 60);
        // Esc once in battle only asks; another key carries on.
        app.key(Key::Back);
        frames(&mut app, 10);
        assert_eq!(app.top_title(), "Battleship");
        // Sweep every square from the top left, row by row, waiting out the
        // handheld's reply after each shot, until one fleet is gone.
        for _ in 0..9 {
            app.key(Key::Up);
            app.key(Key::Left);
        }
        frames(&mut app, 30);
        let games = || {
            let (won, lost) = Record::open().of(Level::Normal);
            won + lost
        };
        let mut shots = 0;
        'sweep: for y in 0..10 {
            for x in 0..10 {
                app.key(Key::Enter);
                shots += 1;
                frames(&mut app, 45);
                if games() == 1 {
                    break 'sweep;
                }
                if x < 9 {
                    app.key(if y % 2 == 0 { Key::Right } else { Key::Left });
                } else {
                    app.key(Key::Down);
                }
            }
        }
        assert_eq!(games(), 1, "the game ended after {shots} shots, and was kept");
        assert!(shots >= 17, "no fleet goes down in fewer than 17 shots");
        // The result comes in; Enter plays again, Esc goes back to the levels.
        frames(&mut app, 120);
        assert!(app.ui.missing_icons().is_empty(), "{:?}", app.ui.missing_icons());
        app.key(Key::Enter);
        frames(&mut app, 60);
        assert_eq!(app.top_title(), "Battleship");
        app.key(Key::Back);
        frames(&mut app, 60);
        assert_eq!(app.top_title(), "Levels");
        app.key(Key::Back);
        assert!(app.done(), "Esc on the levels leaves");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
