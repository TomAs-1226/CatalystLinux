//! Sudoku for Catalyst, rebuilt from sudokuzero (MIT) in the CardputerZero
//! app store: its puzzle maker (one answer, every time), its three levels,
//! its rules (a wrong digit shows at once; the same digit again clears it;
//! pencil notes; how many of each digit are left; the clock starts on the
//! first digit; the best time at each level). Drawn with Catalyst Kit.
//!
//! The board takes the panel's height; notes are drawn as dots where their
//! digit sits on a phone's keypad (a digit 4 px tall could not be read),
//! and spelt out beside the board for the cell in focus.

mod puzzle;

use catalyst_kit::prelude::*;
use catalyst_kit::Spring;
use puzzle::{box_of, col_of, row_of, Level, Puzzle};

/// The board: 16 px cells, a pixel apart within a box, 3 px between boxes.
const CELL: f32 = 16.0;
const LINE: f32 = 1.0;
const BOX_GAP: f32 = 3.0;
const BOARD: f32 = 9.0 * CELL + 6.0 * LINE + 2.0 * BOX_GAP;
const MARGIN: f32 = (H - BOARD) * 0.5;
/// The panel beside the board.
const PANEL_X: f32 = MARGIN + BOARD + 8.0;
const PANEL_W: f32 = W - PANEL_X - MARGIN;

fn cell_rect(i: usize) -> Rect {
    let (r, c) = (row_of(i) as f32, col_of(i) as f32);
    let along = |k: f32| k * (CELL + LINE) + (k / 3.0).floor() * (BOX_GAP - LINE);
    Rect::new(MARGIN + along(c), MARGIN + along(r), CELL, CELL)
}

/// Seconds as a clock: 4:07, or 1:02:07 past the hour.
fn clock(seconds: u32) -> String {
    match seconds {
        s if s >= 3600 => format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
        s => format!("{}:{:02}", s / 60, s % 60),
    }
}

// ------------------------------------------------------------------ game ---

/// A game in play, and how it is kept between runs.
#[derive(Clone, Debug, PartialEq)]
struct Game {
    level: Level,
    puzzle: Puzzle,
    board: [u8; 81],
    /// Pencil notes: bit d for digit d.
    notes: [u16; 81],
    seconds: f32,
}

impl Game {
    fn new(level: Level) -> Game {
        let puzzle = puzzle::make(level, &mut Rng::seeded());
        Game { level, board: puzzle.givens, puzzle, notes: [0; 81], seconds: 0.0 }
    }

    fn given(&self, i: usize) -> bool {
        self.puzzle.givens[i] != 0
    }

    fn wrong(&self, i: usize) -> bool {
        self.board[i] != 0 && self.board[i] != self.puzzle.solution[i]
    }

    fn solved(&self) -> bool {
        self.board == self.puzzle.solution
    }

    fn started(&self) -> bool {
        self.board != self.puzzle.givens || self.notes.iter().any(|n| *n != 0)
    }

    /// How many of `digit` are still to be placed rightly.
    fn left(&self, digit: u8) -> usize {
        9usize.saturating_sub((0..81).filter(|i| self.board[*i] == digit && !self.wrong(*i)).count())
    }

    /// Put `digit` in cell `i`; the same digit again clears it. Placing a
    /// digit rubs it out of the notes it settles in its row, column and box.
    fn place(&mut self, i: usize, digit: u8) {
        if self.given(i) {
            return;
        }
        if self.board[i] == digit {
            self.board[i] = 0;
            return;
        }
        self.board[i] = digit;
        self.notes[i] = 0;
        if digit == self.puzzle.solution[i] {
            for j in 0..81 {
                if row_of(j) == row_of(i) || col_of(j) == col_of(i) || box_of(j) == box_of(i) {
                    self.notes[j] &= !(1 << digit);
                }
            }
        }
    }

    fn note(&mut self, i: usize, digit: u8) {
        if !self.given(i) && self.board[i] == 0 {
            self.notes[i] ^= 1 << digit;
        }
    }

    fn erase(&mut self, i: usize) {
        if !self.given(i) {
            self.board[i] = 0;
            self.notes[i] = 0;
        }
    }

    fn save(&self, saved: &mut Saved) {
        let digits = |g: &[u8; 81]| g.iter().map(|d| char::from(b'0' + d)).collect::<String>();
        saved.set("level", self.level.name());
        saved.set("givens", digits(&self.puzzle.givens));
        saved.set("solution", digits(&self.puzzle.solution));
        saved.set("board", digits(&self.board));
        saved.set("notes", self.notes.iter().map(|n| format!("{n:03x}")).collect::<String>());
        saved.set("seconds", self.seconds as u32);
    }

    fn load(saved: &Saved) -> Option<Game> {
        let digits = |key: &str| -> Option<[u8; 81]> {
            let text = saved.get(key)?;
            let v: Vec<u8> = text.bytes().map(|b| b.wrapping_sub(b'0')).collect();
            (v.len() == 81 && v.iter().all(|d| *d <= 9)).then(|| v.try_into().ok())?
        };
        let notes_text = saved.get("notes")?;
        let notes: Vec<u16> = (0..81).filter_map(|i| u16::from_str_radix(notes_text.get(i * 3..i * 3 + 3)?, 16).ok()).collect();
        let game = Game {
            level: Level::from_name(saved.get("level")?)?,
            puzzle: Puzzle { givens: digits("givens")?, solution: digits("solution")? },
            board: digits("board")?,
            notes: notes.try_into().ok()?,
            seconds: saved.number::<u32>("seconds")? as f32,
        };
        let consistent = (0..81).all(|i| game.puzzle.solution[i] != 0 && (!game.given(i) || game.board[i] == game.puzzle.givens[i]));
        (consistent && !game.solved()).then_some(game)
    }
}

/// The best time at each level, kept in the app's own folder.
fn best(level: Level) -> Option<u32> {
    Saved::open("best").number(level.name())
}

// ------------------------------------------------------------- the menu ---

struct Menu {
    list: List,
    kept: Option<Game>,
}

impl Menu {
    fn new() -> Menu {
        Menu { list: List::default(), kept: Game::load(&Saved::open("game")) }
    }

    /// The rows: the game kept, if there is one, then a new game at each level.
    fn rows(&self) -> usize {
        Level::ALL.len() + usize::from(self.kept.is_some())
    }
}

impl Page for Menu {
    fn title(&self) -> &str {
        "Sudoku"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Flow {
        match key {
            Key::Back => Flow::Quit,
            Key::Enter => {
                let at = self.list.focus;
                let game = match (&self.kept, at) {
                    (Some(game), 0) => game.clone(),
                    (kept, at) => Game::new(Level::ALL[at - usize::from(kept.is_some())]),
                };
                Flow::Push(Box::new(Play::new(game)))
            }
            other => {
                self.list.key(other, self.rows());
                Flow::Stay
            }
        }
    }

    fn resume(&mut self, _cx: &mut Cx) {
        let had = self.kept.is_some();
        self.kept = Game::load(&Saved::open("game"));
        // The kept game finished (or came to be): the focus stays on its level.
        match (had, self.kept.is_some()) {
            (true, false) => self.list.focus = self.list.focus.saturating_sub(1),
            (false, true) => self.list.focus += 1,
            _ => {}
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (top, h, bottom) = (ui.top(), ui.row_h(), ui.bottom());
        let n = self.rows();
        for (i, y) in self.list.layout(ui, n, top, h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, h);
            let focused = i == self.list.focus;
            match (&self.kept, i) {
                (Some(game), 0) => {
                    let left = game.board.iter().filter(|d| **d == 0).count();
                    ui.icon_row(r, "play_arrow", "Carry on", &format!("{} · {left} to go · {}", game.level.name(), clock(game.seconds as u32)), focused);
                }
                (kept, i) => {
                    let level = Level::ALL[i - usize::from(kept.is_some())];
                    let best = best(level).map_or_else(|| "no best yet".to_owned(), |s| format!("best {}", clock(s)));
                    let name = match level {
                        Level::Easy => "New easy puzzle",
                        Level::Standard => "New standard puzzle",
                        Level::Hard => "New hard puzzle",
                    };
                    ui.icon_row(r, "grid_on", name, &best, focused);
                }
            }
        }
        self.list.end(ui, n, top, bottom);
        ui.hint_any(&["enter: play · esc: leave", "enter · esc"]);
    }
}

// ------------------------------------------------------------- the board ---

struct Play {
    game: Game,
    at: usize,
    notes_mode: bool,
    /// The wrong digit's cell, shaking.
    shake: Animated,
    shaking: Option<usize>,
    /// Seconds since it was solved, for the wave across the board.
    solved_for: Option<f32>,
    new_best: bool,
    /// The best before this one, to say what was beaten.
    beaten: Option<u32>,
    /// The clock's last whole second, to draw only when it turns.
    shown_second: u32,
    calm: bool,
}

/// A wrong digit shakes: quick and loose, settled within half a second.
/// (My own tuning: Detent's `tick` stiffness, under-damped further.)
fn shake_spring() -> Spring {
    Spring::stiffness_ratio(900.0, 0.22)
}

/// The wave over the board once it is solved: how long one cell's lift
/// lasts, and how far behind its neighbour each step out from the last digit.
const WAVE_CELL: f32 = 0.42;
const WAVE_STEP: f32 = 0.045;

impl Play {
    fn new(game: Game) -> Play {
        let at = (0..81).find(|i| game.board[*i] == 0).unwrap_or(40);
        Play { shown_second: game.seconds as u32, game, at, notes_mode: false, shake: Animated::new(0.0, shake_spring()).with_rest(0.05), shaking: None, solved_for: None, new_best: false, beaten: None, calm: false }
    }

    fn keep(&self) {
        let mut saved = Saved::open("game");
        if self.game.solved() || !self.game.started() {
            for key in ["level", "givens", "solution", "board", "notes", "seconds"] {
                saved.remove(key);
            }
        } else {
            self.game.save(&mut saved);
        }
        saved.save().ok();
    }

    fn moved(&mut self, dr: i32, dc: i32) {
        let (r, c) = (row_of(self.at) as i32, col_of(self.at) as i32);
        self.at = ((r + dr).rem_euclid(9) * 9 + (c + dc).rem_euclid(9)) as usize;
    }

    fn finish(&mut self, cx: &mut Cx) {
        let seconds = self.game.seconds as u32;
        let mut best = Saved::open("best");
        let before = best.number::<u32>(self.game.level.name());
        self.new_best = before.is_none_or(|b| seconds < b);
        self.beaten = before.filter(|_| self.new_best);
        if self.new_best {
            best.set(self.game.level.name(), seconds);
            best.save().ok();
        }
        self.solved_for = Some(0.0);
        cx.say(if self.new_best { format!("solved in {} · a new best", clock(seconds)) } else { format!("solved in {}", clock(seconds)) });
        self.keep();
    }
}

impl Page for Play {
    fn title(&self) -> &str {
        "Sudoku"
    }

    fn full_screen(&self) -> bool {
        true
    }

    /// Letters reach the board as letters: Backspace erases here, and F X Z
    /// C (and h j k l) move the cursor as on every other screen.
    fn wants_text(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        self.calm = cx.calm();
        if self.solved_for.is_some() {
            return match key {
                Key::Enter => {
                    *self = Play::new(Game::new(self.game.level));
                    Flow::Stay
                }
                Key::Back => Flow::Pop,
                _ => Flow::Stay,
            };
        }
        match key {
            Key::Back => {
                self.keep();
                return Flow::Pop;
            }
            Key::Up | Key::Char('f') | Key::Char('k') => self.moved(-1, 0),
            Key::Down | Key::Char('x') | Key::Char('j') => self.moved(1, 0),
            Key::Left | Key::Char('z') | Key::Char('h') => self.moved(0, -1),
            Key::Right | Key::Char('c') | Key::Char('l') => self.moved(0, 1),
            Key::Char('n') | Key::Tab => {
                self.notes_mode = !self.notes_mode;
                cx.say(if self.notes_mode { "notes on" } else { "notes off" });
            }
            Key::Backspace | Key::Delete | Key::Char('0') => self.game.erase(self.at),
            Key::Char(d @ '1'..='9') => {
                let digit = d as u8 - b'0';
                if self.game.given(self.at) {
                    cx.say("that digit is given");
                } else if self.notes_mode {
                    self.game.note(self.at, digit);
                } else {
                    self.game.place(self.at, digit);
                    if self.game.wrong(self.at) && !cx.calm() {
                        self.shaking = Some(self.at);
                        self.shake.jump(0.0);
                        self.shake.nudge(170.0);
                    }
                    if self.game.solved() {
                        self.finish(cx);
                    }
                }
            }
            _ => {}
        }
        Flow::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.shake.tick(dt);
        if self.shake.is_settled() {
            self.shaking = None;
        }
        if let Some(t) = &mut self.solved_for {
            *t += dt;
            moving |= *t < WAVE_CELL + 14.0 * WAVE_STEP && !self.calm;
        } else if self.game.started() {
            self.game.seconds += dt;
        }
        let second = self.game.seconds as u32;
        let turned = second != self.shown_second;
        self.shown_second = second;
        moving || turned
    }

    fn draw(&mut self, ui: &mut Ui) {
        let t = ui.tokens;
        let ty = ui.ty;
        let game = &self.game;
        let here = game.board[self.at];
        let radius = ui.radius(4.0);
        // The wave once solved: each cell lifts in turn, out from the last digit.
        let lift = |i: usize| -> f32 {
            let Some(since) = self.solved_for else { return 0.0 };
            let d = (row_of(i) as f32 - row_of(self.at) as f32).abs().max((col_of(i) as f32 - col_of(self.at) as f32).abs());
            let local = (since - d * WAVE_STEP) / WAVE_CELL;
            if (0.0..1.0).contains(&local) {
                (local * std::f32::consts::PI).sin()
            } else {
                0.0
            }
        };
        for i in 0..81 {
            let r = cell_rect(i);
            let same_house = row_of(i) == row_of(self.at) || col_of(i) == col_of(self.at) || box_of(i) == box_of(self.at);
            let mut fill = if same_house { t.surface1.mix(t.surface2, 0.55) } else { t.surface1 };
            if here != 0 && game.board[i] == here && !game.wrong(i) {
                fill = t.surface2.mix(t.ice, 0.2);
            }
            let up = lift(i);
            fill = fill.mix(t.ice, 0.35 * up);
            ui.surface(r.inset(-up), radius, fill);
            let digit = game.board[i];
            if digit != 0 {
                let style = if game.given(i) { TextStyle::sans(ty.name.size, 650.0) } else { TextStyle::sans(ty.name.size, 480.0) };
                let ink = if game.wrong(i) {
                    ui.fault()
                } else if game.given(i) {
                    t.ink
                } else {
                    ui.good()
                };
                let dx = if self.shaking == Some(i) { self.shake.value() * 0.06 } else { 0.0 };
                ui.text_at(r.cx() + dx, r.cy(), Align::Center, &digit.to_string(), style, ink);
            } else if game.notes[i] != 0 {
                for d in 1..=9u16 {
                    if game.notes[i] & (1 << d) != 0 {
                        let (nr, nc) = ((d - 1) / 3, (d - 1) % 3);
                        ui.dot(r.x + 4.0 + nc as f32 * 4.0, r.y + 4.0 + nr as f32 * 4.0, 1.2, t.dim);
                    }
                }
            }
        }
        if self.solved_for.is_none() {
            ui.focus_ring(cell_rect(self.at), radius);
        }
        self.draw_panel(ui);
    }
}

impl Play {
    fn draw_panel(&mut self, ui: &mut Ui) {
        let (t, ty) = (ui.tokens, ui.ty);
        let game = &self.game;
        let (x, w) = (PANEL_X, PANEL_W);
        // The level, and notes when they are on.
        let cap_cy = MARGIN + ty.caption.size * 0.5 + 1.0;
        let notes = game.notes[self.at];
        let notes_line = (notes != 0 && game.board[self.at] == 0 && self.solved_for.is_none())
            .then(|| format!("notes {}", (1..=9).filter(|d| notes & (1 << d) != 0).map(|d: u16| d.to_string()).collect::<Vec<_>>().join(" ")));
        if self.notes_mode {
            let chip_w = ui.measure("notes", ty.caption) + 12.0;
            ui.chip(x + w - chip_w, cap_cy, "notes", t.ice, t.on_ice);
        }
        // The clock, large.
        let display = ty.display(30.0);
        let baseline = (cap_cy + ty.caption.size * 0.5 + 4.0 + display.size * 0.78).round();
        let time = clock(game.seconds as u32);
        let tw = ui.text(x, baseline, &time, display, t.ink);
        if let Some(best) = best(game.level).filter(|_| self.solved_for.is_none()) {
            let room = w - tw - 8.0;
            let line = format!("best {}", clock(best));
            if ui.measure(&line, ty.caption) <= room {
                ui.text(x + tw + 6.0, baseline, &line, ty.caption, t.dim);
            }
        }
        let hint_cy = H - MARGIN - ty.caption.size * 0.5;
        if self.solved_for.is_some() {
            ui.text_at(x + 1.0, cap_cy, Align::Left, game.level.name(), ty.caption, t.dim);
            // Solved: what it took, and the way on.
            let line_cy = baseline + 6.0 + ty.name.size * 0.6;
            let words = if self.new_best { "a new best" } else { "solved" };
            ui.text_fit(x, line_cy, words, ty.name, if self.new_best { t.signal } else { t.ink }, w);
            // A new best says what it beat; otherwise, the best to beat.
            let after = match (self.new_best, self.beaten, best(game.level)) {
                (true, Some(was), _) => format!("the last was {}", clock(was)),
                (true, None, _) => String::new(),
                (false, _, Some(b)) => format!("best {}", clock(b)),
                (false, _, None) => String::new(),
            };
            ui.text_fit(x, line_cy + ty.name.size * 0.6 + 4.0 + ty.caption.size * 0.5, &after, ty.caption, t.dim, w);
            let bh = ui.row_h();
            let button = Rect::new(x, hint_cy - ty.caption.size * 0.5 - 6.0 - bh, w, bh);
            ui.button(button, "replay", "Next puzzle", true, true);
            fit_hint(ui, x, w, hint_cy, &["enter: next · esc: menu", "enter · esc"]);
            return;
        }
        // How many of each digit are left: a keypad of nine.
        let tile_h = (ty.name.size + 7.0).round();
        let gap = 3.0;
        let tile_w = ((w - 2.0 * gap) / 3.0).floor();
        let tally_y = hint_cy - ty.caption.size * 0.5 - 5.0 - (3.0 * tile_h + 2.0 * gap);
        let here = game.board[self.at];
        for d in 1..=9u8 {
            let (r, c) = ((d - 1) / 3, (d - 1) % 3);
            let tile = Rect::new(x + c as f32 * (tile_w + gap), tally_y + r as f32 * (tile_h + gap), tile_w, tile_h);
            let left = game.left(d);
            ui.surface(tile, ui.radius(6.0), if here == d { t.surface2 } else { t.surface1 });
            ui.text_at(tile.x + 8.0, tile.cy(), Align::Left, &d.to_string(), ty.name, if left == 0 { t.faint } else { t.ink });
            if left == 0 {
                ui.icon("check_circle", true, tile.right() - 10.0, tile.cy(), 13.0, ui.good());
            } else {
                ui.text_at(tile.right() - 7.0, tile.cy(), Align::Right, &left.to_string(), ty.caption, t.dim);
            }
        }
        // The notes of the cell in focus, spelt out: between the clock and
        // the digits where there is room, else in the level's place.
        let room = tally_y - (baseline + 3.0);
        match notes_line {
            Some(line) if room >= ty.caption.size + 6.0 => {
                ui.text_fit(x + 1.0, (baseline + 3.0 + tally_y) * 0.5, &line, ty.caption, t.dim, w);
                ui.text_at(x + 1.0, cap_cy, Align::Left, game.level.name(), ty.caption, t.dim);
            }
            Some(line) => {
                ui.text_fit(x + 1.0, cap_cy, &line, ty.caption, t.ink, w - 50.0);
            }
            None => {
                ui.text_at(x + 1.0, cap_cy, Align::Left, game.level.name(), ty.caption, t.dim);
            }
        }
        fit_hint(ui, x, w, hint_cy, &["n notes · 0 erase · esc", "n notes · esc", "esc"]);
    }
}

/// A hint under the panel: the longest of `choices` that fits its width.
fn fit_hint(ui: &mut Ui, x: f32, w: f32, cy: f32, choices: &[&str]) {
    let (style, color) = (ui.ty.caption, ui.tokens.dim);
    let line = choices.iter().find(|c| ui.measure(c, style) <= w).or(choices.last()).copied().unwrap_or("");
    ui.text_at(x + w * 0.5, cy, Align::Center, line, style, color);
}

// ------------------------------------------------------------------- main ---

/// The arrows from one cell to another.
fn path(from: usize, to: usize) -> Vec<Key> {
    let (dr, dc) = (row_of(to) as i32 - row_of(from) as i32, col_of(to) as i32 - col_of(from) as i32);
    let mut keys: Vec<Key> = std::iter::repeat_n(if dr > 0 { Key::Down } else { Key::Up }, dr.unsigned_abs() as usize).collect();
    keys.extend(std::iter::repeat_n(if dc > 0 { Key::Right } else { Key::Left }, dc.unsigned_abs() as usize));
    keys
}

/// A game played through from the menu, as keys, scene by scene: the
/// first digit, a wrong one, notes, every digit but the last, the last,
/// and back to the menu. `game` is the easy deal the menu's first row makes.
fn script(game: &Game) -> Vec<(&'static str, Vec<Key>)> {
    let empty: Vec<usize> = (0..81).filter(|i| game.board[*i] == 0).collect();
    let digit = |i: usize| Key::Char((b'0' + game.puzzle.solution[i]) as char);
    let (first, second, last) = (empty[0], empty[1], empty[empty.len() - 1]);
    let wrong = {
        let right = game.puzzle.solution[second];
        Key::Char((b'0' + if right == 9 { 1 } else { right + 1 }) as char)
    };
    let mut rest = Vec::new();
    let mut at = second;
    for &i in &empty[1..empty.len() - 1] {
        rest.extend(path(at, i));
        rest.push(digit(i));
        at = i;
    }
    rest.extend(path(at, last));
    vec![
        ("1-menu", vec![]),
        ("2-board", vec![Key::Enter, digit(first)]),
        ("3-wrong-digit", path(first, second).into_iter().chain([wrong]).collect()),
        ("4-notes", vec![Key::Char('0'), Key::Char('n'), Key::Char('2'), Key::Char('5'), Key::Char('7'), Key::Char('n')]),
        ("5-one-left", rest),
        ("6-solved", vec![digit(last)]),
        ("7-menu-best", vec![Key::Back]),
    ]
}

fn main() {
    if !std::env::args().any(|a| a == "--shots") {
        return catalyst_kit::launch(Menu::new, &[]);
    }
    // The gallery's deal is the same every time (it fixes the seed), so the
    // keys that solve it can be worked out here. A fresh folder: no best yet.
    std::env::set_var("CATALYST_SEED", "5805");
    let data = std::env::temp_dir().join("catalyst-sudoku-gallery");
    std::fs::remove_dir_all(&data).ok();
    std::env::set_var("CATALYST_APP_DATA", &data);
    let scenes = script(&Game::new(Level::Easy));
    let scenes: Vec<(&str, &[Key])> = scenes.iter().map(|(n, k)| (*n, k.as_slice())).collect();
    catalyst_kit::launch(Menu::new, &scenes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalyst_kit::Runner;

    /// The app's folder is named in the environment, which every test
    /// shares: one test at a time has it.
    static FOLDER: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn data(name: &str) -> std::sync::MutexGuard<'static, ()> {
        let held = FOLDER.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("sudoku-test-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::env::set_var("CATALYST_APP_DATA", dir);
        held
    }

    #[test]
    fn the_board_fits_the_panel_with_room_for_its_panel() {
        let last = cell_rect(80);
        assert!(last.bottom() <= H - MARGIN + 0.01 && cell_rect(0).y >= MARGIN - 0.01);
        assert!(PANEL_W >= 130.0, "{PANEL_W}");
        // Boxes are set apart: the gap between cells 2 and 3 is the box gap.
        assert_eq!(cell_rect(3).x - cell_rect(2).right(), BOX_GAP);
        assert_eq!(cell_rect(1).x - cell_rect(0).right(), LINE);
    }

    #[test]
    fn digits_go_in_come_out_and_settle_notes() {
        let mut game = Game::new(Level::Standard);
        let empty: Vec<usize> = (0..81).filter(|i| game.board[*i] == 0).collect();
        let (a, b) = (empty[0], *empty.iter().find(|j| row_of(**j) == row_of(empty[0]) && **j != empty[0]).unwrap_or(&empty[1]));
        let right = game.puzzle.solution[a];
        game.note(b, right);
        game.place(a, right);
        assert_eq!(game.notes[b] & (1 << right), 0, "a right digit rubs out its notes in its row");
        assert!(!game.wrong(a));
        game.place(a, right);
        assert_eq!(game.board[a], 0, "the same digit again clears it");
        let wrong = if right == 9 { 1 } else { right + 1 };
        game.place(a, wrong);
        assert!(game.wrong(a));
        let given = (0..81).find(|i| game.given(*i)).unwrap();
        game.place(given, 1);
        game.erase(given);
        assert_eq!(game.board[given], game.puzzle.givens[given], "a given stays");
        assert_eq!(game.left(right) + (0..81).filter(|i| game.board[*i] == right && !game.wrong(*i)).count(), 9);
    }

    #[test]
    fn a_game_is_kept_and_read_back() {
        let _folder = data("kept");
        let mut game = Game::new(Level::Hard);
        let i = (0..81).find(|i| game.board[*i] == 0).unwrap();
        game.note(i, 3);
        game.seconds = 125.0;
        let mut saved = Saved::open("game");
        game.save(&mut saved);
        saved.save().unwrap();
        assert_eq!(Game::load(&Saved::open("game")), Some(game));
    }

    #[test]
    fn a_whole_game_plays_to_its_end_through_the_keys() {
        let _folder = data("play");
        std::env::set_var("CATALYST_SEED", "99");
        let game = Game::new(Level::Easy);
        let mut app = Runner::new(Box::new(Menu::new()), Look::default());
        let frames = |app: &mut Runner, n: usize| {
            for _ in 0..n {
                if app.tick(1.0 / 40.0) {
                    app.draw();
                }
            }
        };
        frames(&mut app, 30);
        let scenes = script(&game);
        for (name, keys) in &scenes {
            for key in keys {
                app.key(*key);
                frames(&mut app, 2);
            }
            frames(&mut app, 30);
            if *name == "3-wrong-digit" {
                // Left part-way, then carried on: the game is kept.
                assert!(Game::load(&Saved::open("game")).is_none(), "kept only when the board is left");
            }
        }
        assert!(best(Level::Easy).is_some(), "the time is kept as the best");
        assert!(Game::load(&Saved::open("game")).is_none(), "a solved game is not kept to carry on");
        assert!(app.ui.missing_icons().is_empty(), "{:?}", app.ui.missing_icons());
        app.key(Key::Back);
        assert!(app.done());
    }
}
