//! Typing: how fast the handheld's keyboard goes under your thumbs.
//!
//! A native rebuild of key-dash (taM5CZTyping, by yuk1-kondo, MIT) with
//! Catalyst Kit. Type the word shown, letter by letter, against the clock:
//! a 30- or 60-second sprint, or ten words. Words a minute, accuracy, and
//! the best for each round and word list (everyday English, code, or the
//! pit's own), kept in the app's folder.
//!
//! - [`model`]: the round as key-dash counts it, its clock passed in.
//! - [`words`]: the three lists.
//! - [`records`]: the bests, and the round and list last chosen.
//! - [`start`]: the first page; [`play`]: a round and its results.

mod model;
mod play;
mod records;
mod start;
mod words;

use catalyst_kit::{Key, Rng};

use model::{Model, Round};
use start::Start;
use words::Pack;

/// The seed the kit's gallery deals with (it sets `CATALYST_SEED` to it), so
/// the gallery's keys below can type the words it will deal.
const GALLERY_SEED: u64 = 5805;

/// How many keys fill a 30-second sprint in the gallery, which runs a
/// second and a half of frames after every key.
const GALLERY_KEYS_PER_SPRINT: usize = 20;

/// Keys that play a known round as a person would: a model of the same
/// deal, typed alongside.
struct Typist {
    model: Model,
    /// Keys typed into the round so far.
    keys: usize,
}

impl Typist {
    /// The round the start page deals first from `rng`.
    fn new(round: Round, pack: Pack, rng: &mut Rng) -> Typist {
        Typist { model: Model::new(round, pack, start::seed_from(rng)), keys: 0 }
    }

    fn press(&mut self, c: char, out: &mut Vec<Key>) {
        self.model.type_char(c, 0);
        self.keys += 1;
        out.push(Key::Char(c));
    }

    /// The next `n` letters, right, on into the next word if need be.
    fn letters(&mut self, n: usize, out: &mut Vec<Key>) {
        for _ in 0..n {
            let c = self.model.word().as_bytes()[self.model.typed()] as char;
            self.press(c, out);
        }
    }

    fn rest_of_word(&mut self, out: &mut Vec<Key>) {
        let n = self.model.word().len() - self.model.typed();
        self.letters(n, out);
    }

    /// A key that is not the next letter.
    fn wrong(&mut self, out: &mut Vec<Key>) {
        let want = self.model.word().as_bytes()[self.model.typed()] as char;
        let c = ['q', 'j', 'v'].into_iter().find(|c| *c != want).unwrap_or('k');
        self.press(c, out);
    }
}

/// The gallery: the start page with the pit's words chosen, a word half
/// typed, a wrong key, the round under way, its results, and the best kept.
fn gallery_scenes() -> Vec<(&'static str, Vec<Key>)> {
    let mut typist = Typist::new(Round::Sprint30, Pack::Pit, &mut Rng::new(GALLERY_SEED));
    let mut half = vec![Key::Enter];
    let n = typist.model.word().len().div_ceil(2);
    typist.letters(n, &mut half);
    let mut wrong = Vec::new();
    typist.wrong(&mut wrong);
    // The rest of that word, then all of the next and the third begun, as
    // far as leaves some of the clock.
    let mut under_way = Vec::new();
    typist.rest_of_word(&mut under_way);
    let more = (typist.model.word().len() + 1).min(16usize.saturating_sub(typist.keys));
    typist.letters(more, &mut under_way);
    // Typing on until the clock runs out with the last of these keys, then
    // a moment for the results to arrive.
    let mut results = Vec::new();
    while typist.keys < GALLERY_KEYS_PER_SPRINT {
        typist.letters(1, &mut results);
    }
    results.extend([Key::Tab, Key::Tab]);
    vec![
        ("1-start", vec![Key::Right, Key::Right]),
        ("2-half-a-word", half),
        ("3-wrong-key", wrong),
        ("4-under-way", under_way),
        ("5-results", results),
        ("6-best-kept", vec![Key::Back]),
    ]
}

fn main() {
    // The gallery plays in a folder of its own, so it starts with no bests
    // and leaves none behind.
    let shots = cfg!(not(target_arch = "aarch64")) && std::env::args().any(|a| a == "--shots");
    let scratch = shots.then(|| {
        let dir = std::env::temp_dir().join(format!("catalyst-typing-gallery-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::env::set_var("CATALYST_APP_DATA", &dir);
        dir
    });
    let owned = if shots { gallery_scenes() } else { Vec::new() };
    let scenes: Vec<(&str, &[Key])> = owned.iter().map(|(name, keys)| (*name, keys.as_slice())).collect();
    catalyst_kit::launch(Start::new, &scenes);
    if let Some(dir) = scratch {
        std::fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalyst_kit::{Look, Runner, TextSize, Ui, PAD, W};

    use crate::model::{Phase, PRACTICE_WORDS};
    use crate::records::Records;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("catalyst-typing-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    fn frames(app: &mut Runner, n: usize) {
        for _ in 0..n {
            if app.tick(1.0 / 60.0) {
                app.draw();
            }
        }
    }

    /// A key into the app and into a model of the same round, 0.2 s apart.
    fn send(app: &mut Runner, shadow: &mut Model, now: &mut u64, key: Key) {
        match key {
            Key::Char(c) => {
                shadow.type_char(c, *now);
            }
            Key::Backspace => {
                shadow.backspace(*now);
            }
            _ => {}
        }
        app.key(key);
        frames(app, 12);
        *now += 200;
    }

    #[test]
    fn a_whole_round_is_played_with_keys() {
        let dir = scratch("runner");
        let mut app = Runner::new(Box::new(Start::at(&dir, Rng::new(GALLERY_SEED))), Look::default());
        frames(&mut app, 60);
        assert_eq!(app.top_title(), "Typing");
        assert!(!app.animating(), "a page at rest draws nothing");
        // The ten-word round, in the pit's words.
        for key in [Key::Down, Key::Down, Key::Right, Key::Right, Key::Enter] {
            app.key(key);
        }
        frames(&mut app, 60);
        assert_eq!(app.top_title(), "10 words");
        assert!(!app.animating(), "the clock waits for the first key");

        // A key every 0.2 s: one wrong key in the second word, and a letter
        // taken back and typed again in the third.
        let mut shadow = Model::new(Round::Words10, Pack::Pit, start::seed_from(&mut Rng::new(GALLERY_SEED)));
        let mut now = 0;
        let mut letters = 0;
        for n in 0..PRACTICE_WORDS {
            let first = shadow.word().as_bytes()[0] as char;
            if n == 1 {
                send(&mut app, &mut shadow, &mut now, Key::Char(if first == 'q' { 'j' } else { 'q' }));
            }
            if n == 2 {
                send(&mut app, &mut shadow, &mut now, Key::Char(first));
                send(&mut app, &mut shadow, &mut now, Key::Backspace);
            }
            let word = shadow.word();
            letters += word.len() as u32;
            for c in word.chars() {
                send(&mut app, &mut shadow, &mut now, Key::Char(c));
                assert!(app.animating(), "the clock is shown running, and the results arriving");
            }
        }
        assert_eq!(shadow.phase(), Phase::Done);
        frames(&mut app, 90);
        assert_eq!(app.top_title(), "Results");
        assert!(!app.animating(), "the results at rest draw nothing");
        let done = shadow.summary();
        // Attempts: every letter, the wrong key, and the letter taken back.
        assert_eq!(done.accuracy, (letters + 1) * 100 / (letters + 2));
        // The best is kept: the shadow's speed, give or take the frames' rounding.
        let best = Records::in_dir(&dir).best(Round::Words10, Pack::Pit);
        assert!(done.wpm > 0 && best.abs_diff(done.wpm) <= 1, "kept {best}, typed {}", done.wpm);

        // Enter plays again; Esc goes back to the rounds; Esc there leaves.
        app.key(Key::Enter);
        frames(&mut app, 60);
        assert_eq!(app.top_title(), "10 words");
        app.key(Key::Back);
        frames(&mut app, 60);
        assert_eq!(app.top_title(), "Typing");
        assert!(app.ui.missing_icons().is_empty(), "{:?}", app.ui.missing_icons());
        app.key(Key::Back);
        assert!(app.done());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_gallery_plays_the_round_it_shows() {
        let dir = scratch("gallery");
        for look in [Look::default(), Look { calm: true, text: TextSize::Large, ..Look::default() }] {
            std::fs::remove_dir_all(&dir).ok();
            let mut app = Runner::new(Box::new(Start::at(&dir, Rng::new(GALLERY_SEED))), look);
            frames(&mut app, 90);
            let titles = ["Typing", "30 s sprint", "30 s sprint", "30 s sprint", "Results", "Typing"];
            for ((name, keys), title) in gallery_scenes().iter().zip(titles) {
                for key in keys {
                    app.key(*key);
                    frames(&mut app, 90);
                }
                assert_eq!(app.top_title(), title, "after {name}");
            }
            // Nineteen letters right and one wrong in the thirty seconds.
            assert_eq!(Records::in_dir(&dir).best(Round::Sprint30, Pack::Pit), 19 * 12_000 / 30_000);
            assert!(app.ui.missing_icons().is_empty(), "{:?}", app.ui.missing_icons());
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_word_fits_the_panel_at_the_largest_text() {
        let mut ui = Ui::new(Look { text: TextSize::Large, ..Look::default() });
        let style = ui.ty.display(play::WORD_SIZE);
        for pack in Pack::ALL {
            for word in pack.words() {
                assert!(ui.measure(word, style) + 12.0 < W - 2.0 * PAD, "{word}");
            }
        }
    }
}
