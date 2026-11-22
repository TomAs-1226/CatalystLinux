//! New passwords: random characters (a length, with signs or without), or
//! a phrase of words from the built-in list. Every pick is uniform, from
//! the kernel's random numbers, with no bias from a remainder.

use zeroize::Zeroizing;

use super::crypto;
use super::words::WORDS;

const LOWER: &[u8] = b"abcdefghijkmnopqrstuvwxyz";
const UPPER: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const DIGITS: &[u8] = b"23456789";
/// Signs that need no Shift on a US keyboard's own row, or are easy to find,
/// and that sites take.
const SIGNS: &[u8] = b"!#$%&*+-=?@^_~";

/// What to make.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Recipe {
    /// `len` characters: letters and digits, and signs if `signs`.
    Chars { len: usize, signs: bool },
    /// `words` words from the list, joined by `-`, one of them with a digit.
    Words { words: usize },
}

impl Default for Recipe {
    fn default() -> Self {
        Recipe::Chars { len: 20, signs: true }
    }
}

pub const LEN_RANGE: (usize, usize) = (8, 64);
pub const WORDS_RANGE: (usize, usize) = (3, 10);

impl Recipe {
    /// How hard what it makes is to guess, in bits (for someone who knows
    /// the recipe).
    pub fn bits(self) -> f32 {
        match self {
            Recipe::Chars { len, signs } => len as f32 * (alphabet(signs).len() as f32).log2(),
            Recipe::Words { words } => words as f32 * (WORDS.len() as f32).log2() + (DIGITS.len() as f32).log2() + (words as f32).log2(),
        }
    }

    /// In a word: what the bits come to (for a site's password, which is
    /// only ever guessed against the site or its leaked hashes).
    pub fn strength(self) -> &'static str {
        match self.bits() {
            b if b >= 90.0 => "very strong",
            b if b >= 60.0 => "strong",
            b if b >= 45.0 => "fair",
            _ => "weak",
        }
    }
}

fn alphabet(signs: bool) -> Vec<u8> {
    let mut a = [LOWER, UPPER, DIGITS].concat();
    if signs {
        a.extend_from_slice(SIGNS);
    }
    a
}

/// A number below `n`, uniform: random 32-bit words above the largest
/// multiple of `n` are drawn again.
pub fn below(n: u32) -> Result<u32, crypto::Error> {
    assert!(n > 0);
    let zone = u32::MAX - (u32::MAX % n);
    loop {
        let mut b = [0u8; 4];
        crypto::random(&mut b)?;
        let v = u32::from_le_bytes(b);
        if v < zone {
            return Ok(v % n);
        }
    }
}

/// A new password by `recipe`.
pub fn make(recipe: Recipe) -> Result<Zeroizing<String>, crypto::Error> {
    match recipe {
        Recipe::Chars { len, signs } => {
            let len = len.clamp(LEN_RANGE.0, LEN_RANGE.1);
            let a = alphabet(signs);
            // Drawn again until it has one of each kind asked for (rare at
            // these lengths, and leaves the picks uniform among those that do).
            loop {
                let mut out = Zeroizing::new(String::with_capacity(len));
                for _ in 0..len {
                    out.push(a[below(a.len() as u32)? as usize] as char);
                }
                let has = |set: &[u8]| out.bytes().any(|b| set.contains(&b));
                if has(LOWER) && has(UPPER) && has(DIGITS) && (!signs || has(SIGNS)) {
                    return Ok(out);
                }
            }
        }
        Recipe::Words { words } => {
            let words = words.clamp(WORDS_RANGE.0, WORDS_RANGE.1);
            let digit_at = below(words as u32)? as usize;
            let mut out = Zeroizing::new(String::with_capacity(words * 8));
            for i in 0..words {
                if i > 0 {
                    out.push('-');
                }
                out.push_str(WORDS[below(WORDS.len() as u32)? as usize]);
                if i == digit_at {
                    out.push(DIGITS[below(DIGITS.len() as u32)? as usize] as char);
                }
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_word_list_is_a_thousand_and_twenty_four_plain_words() {
        assert_eq!(WORDS.len(), 1024);
        let mut sorted = WORDS.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 1024, "no word twice");
        assert!(WORDS.iter().all(|w| (3..=6).contains(&w.len()) && w.bytes().all(|b| b.is_ascii_lowercase())));
    }

    #[test]
    fn passwords_have_the_length_and_kinds_asked_for() {
        for (len, signs) in [(8, false), (20, true), (64, true), (2, false), (500, true)] {
            let p = make(Recipe::Chars { len, signs }).unwrap();
            assert_eq!(p.len(), len.clamp(8, 64));
            assert_eq!(p.bytes().any(|b| SIGNS.contains(&b)), signs, "{len} {signs}");
            assert!(p.bytes().any(|b| b.is_ascii_digit()) && p.bytes().any(|b| b.is_ascii_uppercase()));
            assert!(!p.contains(['0', 'O', '1', 'l', 'I']), "no look-alikes: {}", p.as_str());
        }
        let a = make(Recipe::default()).unwrap();
        let b = make(Recipe::default()).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn a_passphrase_is_words_from_the_list_with_one_digit() {
        let p = make(Recipe::Words { words: 5 }).unwrap();
        let parts: Vec<&str> = p.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(p.chars().filter(char::is_ascii_digit).count(), 1);
        for part in parts {
            let word = part.trim_end_matches(|c: char| c.is_ascii_digit());
            assert!(WORDS.contains(&word), "{word}");
        }
        assert!(Recipe::Words { words: 5 }.bits() > 50.0);
        assert!(Recipe::Chars { len: 20, signs: true }.bits() > 120.0);
        assert_eq!((Recipe::Words { words: 3 }.strength(), Recipe::Words { words: 5 }.strength(), Recipe::Words { words: 7 }.strength()), ("weak", "fair", "strong"));
    }

    #[test]
    fn picks_are_uniform() {
        // 6000 picks below 6: each about a sixth (well inside chance).
        let mut counts = [0u32; 6];
        for _ in 0..6000 {
            counts[below(6).unwrap() as usize] += 1;
        }
        assert!(counts.iter().all(|c| (850..=1150).contains(c)), "{counts:?}");
    }
}
