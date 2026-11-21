//! A round of typing, as key-dash counts it, with the clock passed in (in
//! milliseconds) so a test can say exactly when every key landed.
//!
//! - A key counts only if it is the next letter of the word (either case).
//!   A wrong key does not move on, but it is an attempt: accuracy is
//!   correct keys × 100 / attempts.
//! - Backspace takes back the last letter typed.
//! - The clock starts on the first key. A sprint ends when its time is up;
//!   the ten-word round ends on its tenth word.
//! - Words a minute: min(999, letters × 12000 / milliseconds), the letters
//!   being those of the finished words and the typed start of this one
//!   (five letters to a word, so erasing and retyping never adds speed).
//! - The next word comes from xorshift32 over the list, never the word
//!   just typed.

use crate::words::Pack;

/// The kinds of round.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Round {
    Sprint30,
    Sprint60,
    Words10,
}

/// How many words the ten-word round is.
pub const PRACTICE_WORDS: u32 = 10;
/// The counters stop here, as key-dash's do.
const MAX_COUNT: u32 = 100_000_000;

impl Round {
    pub const ALL: [Round; 3] = [Round::Sprint30, Round::Sprint60, Round::Words10];

    /// The name kept in the records file.
    pub fn key(self) -> &'static str {
        match self {
            Round::Sprint30 => "sprint30",
            Round::Sprint60 => "sprint60",
            Round::Words10 => "words10",
        }
    }

    pub fn from_key(key: &str) -> Option<Round> {
        Round::ALL.into_iter().find(|r| r.key() == key.trim())
    }

    /// Its name on the start page.
    pub fn name(self) -> &'static str {
        match self {
            Round::Sprint30 => "30-second sprint",
            Round::Sprint60 => "60-second sprint",
            Round::Words10 => "10 words",
        }
    }

    /// Its page's title.
    pub fn title(self) -> &'static str {
        match self {
            Round::Sprint30 => "30 s sprint",
            Round::Sprint60 => "60 s sprint",
            Round::Words10 => "10 words",
        }
    }

    /// Its name where room is short.
    pub fn short(self) -> &'static str {
        match self {
            Round::Sprint30 => "30 s",
            Round::Sprint60 => "60 s",
            Round::Words10 => "10 words",
        }
    }

    /// How long a sprint lasts; 0 for the ten-word round.
    pub fn limit_ms(self) -> u64 {
        match self {
            Round::Sprint30 => 30_000,
            Round::Sprint60 => 60_000,
            Round::Words10 => 0,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Waiting for the first key.
    Ready,
    Running,
    Done,
}

/// What a key did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Typed {
    /// Not a key that counts (or the round is over).
    Ignored,
    /// Not the next letter: counted as an attempt, nothing moves on.
    Wrong,
    /// The next letter.
    Letter,
    /// The last letter of a word: the next word is up.
    Word,
    /// The last letter of the round's last word.
    Finished,
}

/// A finished round's numbers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Summary {
    pub round: Round,
    pub pack: Pack,
    pub wpm: u32,
    pub accuracy: u32,
    pub words: u32,
    pub elapsed_ms: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Model {
    round: Round,
    pack: Pack,
    /// xorshift32's state: never zero.
    state: u32,
    word: &'static str,
    next: &'static str,
    /// How many letters of the word are typed (only right ones are kept).
    typed: usize,
    phase: Phase,
    attempts: u32,
    correct: u32,
    words: u32,
    /// Letters of the finished words.
    committed: u32,
    started_at: u64,
    finished_elapsed: u64,
    last_correct: bool,
}

fn add(value: &mut u32, amount: u32) {
    *value = (*value).saturating_add(amount).min(MAX_COUNT);
}

impl Model {
    /// A round ready for its first key, its words dealt from `seed`.
    pub fn new(round: Round, pack: Pack, seed: u32) -> Model {
        let mut model = Model {
            round,
            pack,
            state: if seed == 0 { 1 } else { seed },
            word: "",
            next: "",
            typed: 0,
            phase: Phase::Ready,
            attempts: 0,
            correct: 0,
            words: 0,
            committed: 0,
            started_at: 0,
            finished_elapsed: 0,
            last_correct: true,
        };
        model.word = model.pick("");
        model.next = model.pick(model.word);
        model
    }

    /// The same round again from the start, the words dealt on from where
    /// the last round left them.
    pub fn restart(&mut self) {
        *self = Model {
            word: self.next,
            typed: 0,
            phase: Phase::Ready,
            attempts: 0,
            correct: 0,
            words: 0,
            committed: 0,
            started_at: 0,
            finished_elapsed: 0,
            last_correct: true,
            ..*self
        };
        self.next = self.pick(self.word);
    }

    /// xorshift32 over the list, until it is not `previous`.
    fn pick(&mut self, previous: &str) -> &'static str {
        let words = self.pack.words();
        loop {
            let mut x = self.state;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.state = x;
            let word = words[(x % words.len() as u32) as usize];
            if word != previous {
                return word;
            }
        }
    }

    pub fn round(&self) -> Round {
        self.round
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    /// The word being typed.
    pub fn word(&self) -> &'static str {
        self.word
    }
    /// The word after it.
    pub fn next(&self) -> &'static str {
        self.next
    }
    /// How many of the word's letters are typed.
    pub fn typed(&self) -> usize {
        self.typed
    }
    /// False after a wrong key, until a right one or Backspace.
    pub fn last_correct(&self) -> bool {
        self.last_correct
    }
    pub fn words_done(&self) -> u32 {
        self.words
    }
    #[cfg(test)]
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    pub fn elapsed_ms(&self, now: u64) -> u64 {
        match self.phase {
            Phase::Done => self.finished_elapsed,
            Phase::Running => now.saturating_sub(self.started_at),
            Phase::Ready => 0,
        }
    }

    /// A sprint's time left (its whole length before the first key).
    pub fn remaining_ms(&self, now: u64) -> u64 {
        let limit = self.round.limit_ms();
        limit - limit.min(self.elapsed_ms(now))
    }

    pub fn accuracy(&self) -> u32 {
        if self.attempts == 0 {
            100
        } else {
            (self.correct as u64 * 100 / self.attempts as u64) as u32
        }
    }

    pub fn wpm(&self, now: u64) -> u32 {
        let elapsed = self.elapsed_ms(now);
        if elapsed == 0 {
            return 0;
        }
        let letters = self.committed as u64 + self.typed as u64;
        (letters * 12_000 / elapsed).min(999) as u32
    }

    /// A key typed at `now`.
    pub fn type_char(&mut self, c: char, now: u64) -> Typed {
        if self.phase == Phase::Done || !(' '..='~').contains(&c) || (self.phase == Phase::Ready && c == ' ') {
            return Typed::Ignored;
        }
        if self.tick(now) {
            return Typed::Ignored;
        }
        if self.phase == Phase::Ready {
            self.phase = Phase::Running;
            self.started_at = now;
        }
        let c = c.to_ascii_lowercase() as u8;
        add(&mut self.attempts, 1);
        self.last_correct = self.word.as_bytes().get(self.typed) == Some(&c);
        if !self.last_correct {
            return Typed::Wrong;
        }
        add(&mut self.correct, 1);
        self.typed += 1;
        if self.typed < self.word.len() {
            return Typed::Letter;
        }
        add(&mut self.words, 1);
        add(&mut self.committed, self.word.len() as u32);
        self.typed = 0;
        if self.round == Round::Words10 && self.words >= PRACTICE_WORDS {
            self.finish(self.elapsed_ms(now));
            return Typed::Finished;
        }
        self.word = self.next;
        self.next = self.pick(self.word);
        Typed::Word
    }

    /// Backspace at `now`: the last letter typed is taken back. True if the
    /// round was running.
    pub fn backspace(&mut self, now: u64) -> bool {
        if self.phase != Phase::Running || self.tick(now) {
            return false;
        }
        self.typed = self.typed.saturating_sub(1);
        self.last_correct = true;
        true
    }

    /// Time passing: true if a sprint's time just ran out.
    pub fn tick(&mut self, now: u64) -> bool {
        let limit = self.round.limit_ms();
        if self.phase == Phase::Running && limit > 0 && self.elapsed_ms(now) >= limit {
            self.finish(limit);
            return true;
        }
        false
    }

    fn finish(&mut self, elapsed: u64) {
        if self.phase == Phase::Running {
            self.finished_elapsed = elapsed;
            self.phase = Phase::Done;
        }
    }

    /// The numbers of a finished round.
    pub fn summary(&self) -> Summary {
        Summary { round: self.round, pack: self.pack, wpm: self.wpm(0), accuracy: self.accuracy(), words: self.words, elapsed_ms: self.elapsed_ms(0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type `s` one key every `step` ms from `at`; returns when the next key would land.
    fn type_str(m: &mut Model, s: &str, at: u64, step: u64) -> u64 {
        let mut now = at;
        for c in s.chars() {
            m.type_char(c, now);
            now += step;
        }
        now
    }

    #[test]
    fn the_picker_never_deals_a_word_twice_running() {
        for pack in Pack::ALL {
            for seed in [1, 2, 5805, 0xdead_beef, u32::MAX] {
                let mut m = Model::new(Round::Sprint60, pack, seed);
                let mut seen = std::collections::HashSet::new();
                let mut last = m.word();
                for _ in 0..2000 {
                    assert_ne!(m.next(), m.word());
                    let word = m.word();
                    type_str(&mut m, word, 0, 0);
                    assert_ne!(m.word(), last, "{pack:?} seed {seed}");
                    assert!(pack.words().contains(&m.word()));
                    seen.insert(m.word());
                    last = m.word();
                }
                assert!(seen.len() > 48, "the deal reaches most of the list: {}", seen.len());
            }
        }
    }

    #[test]
    fn the_deal_is_xorshift32_as_key_dash_does_it() {
        // x ^= x << 13; x ^= x >> 17; x ^= x << 5, from seed 1: 270369, then 67634689.
        let m = Model::new(Round::Sprint30, Pack::English, 1);
        assert_eq!(m.word(), crate::words::ENGLISH[(270_369u32 % 64) as usize]);
        assert_eq!(m.next(), crate::words::ENGLISH[(67_634_689u32 % 64) as usize]);
        // Zero would stick at zero: it is taken as one.
        assert_eq!(Model::new(Round::Sprint30, Pack::English, 0).word(), m.word());
    }

    #[test]
    fn a_scripted_sprint_scores_as_key_dash_does() {
        let mut m = Model::new(Round::Sprint30, Pack::Pit, 77);
        assert_eq!((m.phase(), m.wpm(5_000), m.accuracy(), m.remaining_ms(5_000)), (Phase::Ready, 0, 100, 30_000));
        // Space before the first key does nothing, and the clock waits for a letter.
        assert_eq!(m.type_char(' ', 1_000), Typed::Ignored);
        let first = m.word();
        let len = first.len() as u64;
        // The first word, a key every 200 ms from t = 10 s.
        let mut now = 10_000;
        for (i, c) in first.chars().enumerate() {
            let typed = m.type_char(c.to_ascii_uppercase(), now);
            assert_eq!(typed, if i + 1 == first.len() { Typed::Word } else { Typed::Letter }, "capitals count");
            now += 200;
        }
        assert_eq!(m.phase(), Phase::Running);
        // The last key landed (len - 1) × 200 ms after the first.
        let at = 10_000 + (len - 1) * 200;
        assert_eq!(m.wpm(at), (len * 12_000 / ((len - 1) * 200)).min(999) as u32);
        assert_eq!(m.remaining_ms(at), 30_000 - (len - 1) * 200);
        // A wrong key: an attempt, nothing typed.
        let second = m.word();
        let wrong = if second.starts_with('q') { 'z' } else { 'q' };
        assert_eq!(m.type_char(wrong, now), Typed::Wrong);
        assert!(!m.last_correct());
        assert_eq!(m.typed(), 0);
        assert_eq!(m.accuracy(), (len * 100 / (len + 1)) as u32);
        // Two right letters: they count toward speed before the word is done.
        let two: String = second.chars().take(2).collect();
        type_str(&mut m, &two, now + 100, 100);
        assert!(m.last_correct());
        assert_eq!(m.typed(), 2);
        let t = 20_000;
        assert_eq!(m.wpm(t), ((len + 2) * 12_000 / (t - 10_000)) as u32);
        // Time runs out at 30 s after the first key: the clock stops there.
        assert!(!m.tick(39_999));
        assert!(m.tick(40_000));
        assert_eq!(m.phase(), Phase::Done);
        let s = m.summary();
        assert_eq!((s.elapsed_ms, s.words, s.wpm, s.accuracy), (30_000, 1, ((len + 2) * 12_000 / 30_000) as u32, ((len + 2) * 100 / (len + 3)) as u32));
        // Keys after the end do nothing.
        assert_eq!(m.type_char('a', 40_100), Typed::Ignored);
        assert_eq!(m.summary(), s);
    }

    #[test]
    fn a_key_after_the_time_is_up_ends_the_round_and_is_not_counted() {
        let mut m = Model::new(Round::Sprint30, Pack::Code, 9);
        let c = m.word().chars().next().unwrap();
        m.type_char(c, 0);
        assert_eq!(m.type_char('x', 30_000), Typed::Ignored);
        assert_eq!((m.phase(), m.attempts()), (Phase::Done, 1));
        assert!(!m.backspace(30_500));
    }

    #[test]
    fn backspace_takes_back_the_last_letter() {
        let mut m = Model::new(Round::Sprint60, Pack::English, 3);
        assert!(!m.backspace(0), "nothing to take back before the first key");
        let word = m.word();
        let three: String = word.chars().take(3).collect();
        type_str(&mut m, &three, 0, 100);
        m.type_char('#', 300);
        assert!(!m.last_correct());
        assert!(m.backspace(400));
        assert!(m.last_correct(), "Backspace clears a wrong key");
        assert_eq!(m.typed(), 2);
        // Retyping the letter makes the same speed, never more.
        let c = word.as_bytes()[2] as char;
        m.type_char(c, 500);
        assert_eq!(m.typed(), 3);
        assert_eq!(m.wpm(1_000), 3 * 12_000 / 1_000);
        assert_eq!(m.accuracy(), 4 * 100 / 5);
        // Backspace at the start of a word takes nothing.
        let mut fresh = Model::new(Round::Sprint60, Pack::English, 3);
        fresh.type_char('#', 0);
        assert!(fresh.backspace(10));
        assert_eq!(fresh.typed(), 0);
    }

    #[test]
    fn the_ten_word_round_ends_on_its_tenth_word() {
        let mut m = Model::new(Round::Words10, Pack::Pit, 42);
        let mut now = 0;
        let mut letters = 0;
        for n in 1..=PRACTICE_WORDS {
            let word = m.word();
            letters += word.len() as u64;
            let mut last = Typed::Ignored;
            for c in word.chars() {
                last = m.type_char(c, now);
                now += 250;
            }
            assert_eq!(last, if n == PRACTICE_WORDS { Typed::Finished } else { Typed::Word });
        }
        assert_eq!(m.phase(), Phase::Done);
        let s = m.summary();
        let elapsed = now - 250;
        assert_eq!((s.words, s.accuracy, s.elapsed_ms), (10, 100, elapsed));
        assert_eq!(s.wpm, (letters * 12_000 / elapsed) as u32);
        assert_eq!(m.remaining_ms(now), 0, "no time limit");
        // Again: a fresh round, dealt on from the same stream.
        let before = m.word();
        m.restart();
        assert_eq!((m.phase(), m.words_done(), m.accuracy(), m.wpm(now)), (Phase::Ready, 0, 100, 0));
        assert_ne!(m.word(), before);
        assert_ne!(m.word(), m.next());
    }

    #[test]
    fn speed_tops_out_at_999() {
        let mut m = Model::new(Round::Sprint30, Pack::English, 11);
        for _ in 0..5 {
            let w = m.word();
            type_str(&mut m, w, 0, 0);
        }
        let w = m.word();
        type_str(&mut m, &w[..1], 1, 0);
        assert_eq!(m.wpm(1), 999);
    }
}
