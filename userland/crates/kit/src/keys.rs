//! The handheld's keys, read as the shell reads them: the same names, the
//! same Sym and Fn layers, the same repeat on the same clock. An app gets
//! [`Key`]s and never sees a key code.
//!
//! Outside text entry, F X Z C are the arrows and Backspace is Back, as
//! everywhere on Catalyst; a page that takes text ([`crate::Page::wants_text`])
//! gets the letters instead.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    /// Esc (and Backspace outside text): back a page.
    Back,
    Backspace,
    Tab,
    PageUp,
    PageDown,
    /// Fn + Backspace: the character after the cursor.
    Delete,
    /// Fn + K and Fn + N: the first and last row.
    Home,
    End,
    /// F1 to F12: Fn with the number row (F11 and F12 on O and P).
    F(u8),
    /// Fn + U and Fn + I. The kit sets the backlight with them; apps never see them.
    BrightnessDown,
    BrightnessUp,
    Char(char),
    /// A letter with Ctrl held.
    Ctrl(char),
    /// Esc held for a second: leave the app, whatever it is doing.
    Leave,
}

impl Key {
    /// The two ways out of anything: Ctrl-Q, or Esc held down.
    pub fn leaves(self) -> bool {
        matches!(self, Key::Ctrl('q') | Key::Leave)
    }

    /// The key as a page that is not taking text reads it: F X Z C are the
    /// arrows, Backspace is Back.
    pub fn plain(self) -> Key {
        match self {
            Key::Char('f') => Key::Up,
            Key::Char('x') => Key::Down,
            Key::Char('z') => Key::Left,
            Key::Char('c') => Key::Right,
            Key::Backspace => Key::Back,
            other => other,
        }
    }

    /// A digit key's value, 0 to 9.
    pub fn digit(self) -> Option<u8> {
        match self {
            Key::Char(c) if c.is_ascii_digit() => Some(c as u8 - b'0'),
            _ => None,
        }
    }
}

/// The symbol printed on a key of the handheld's Sym layer, by the code its
/// driver sends (the codes are borrowed from keys the CardputerZero does not
/// have; M5Stack's own system maps them back with a console keymap). Only
/// for the handheld's own keyboard: a USB keyboard's `[` is still `[`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn sym_char(code: u16) -> Option<char> {
    Some(match code {
        26 => '!',
        27 => '@',
        39 => '#',
        40 => '$',
        41 => '%',
        43 => '^',
        51 => '&',
        52 => '*',
        53 => '(',
        94 => ')',
        55 => '~',
        69 => '`',
        70 => '_',
        71 => '-',
        72 => '+',
        73 => '=',
        74 => '[',
        75 => ']',
        76 => '{',
        77 => '}',
        79 => ';',
        80 => ':',
        81 => '\'',
        82 => '"',
        83 => '<',
        85 => '>',
        86 => '\\',
        89 => '|',
        90 => ',',
        91 => '.',
        92 => '/',
        93 => '?',
        _ => return None,
    })
}

/// The Fn layer's keys by key code: Delete, Home and End, F1 to F12, brightness.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn fn_key(code: u16) -> Option<Key> {
    Some(match code {
        111 => Key::Delete,
        102 => Key::Home,
        107 => Key::End,
        59..=68 => Key::F((code - 58) as u8),
        87 => Key::F(11),
        88 => Key::F(12),
        224 => Key::BrightnessDown,
        225 => Key::BrightnessUp,
        _ => return None,
    })
}

/// How long Esc is held before it means [`Key::Leave`].
pub const HOLD_TO_LEAVE: Duration = Duration::from_secs(1);
/// How long a key is held before it repeats, and then how often: the shell's.
pub const REPEAT_AFTER: Duration = Duration::from_millis(400);
pub const REPEAT_EVERY: Duration = Duration::from_millis(75);

/// Watches one key being held down.
#[derive(Default)]
pub struct Hold {
    since: Option<Instant>,
    fired: bool,
}

impl Hold {
    /// Whether the key is down now; true, once, when it has been down long enough.
    pub fn update(&mut self, down: bool) -> bool {
        match (down, self.since) {
            (false, _) => {
                self.since = None;
                self.fired = false;
            }
            (true, None) => self.since = Some(Instant::now()),
            (true, Some(since)) if !self.fired && since.elapsed() >= HOLD_TO_LEAVE => {
                self.fired = true;
                return true;
            }
            _ => {}
        }
        false
    }
}

/// Held keys repeat on the kit's own clock, as on the shell's: once when
/// pressed, then after 0.4 s about 13 times a second. Only keys that move or
/// type repeat; Enter, Esc, Tab and the shortcuts count once a press. `Id`
/// names a physical key (a key code).
pub struct Repeat<Id> {
    held: Option<(Id, Key, Instant)>,
}

impl<Id> Default for Repeat<Id> {
    fn default() -> Self {
        Repeat { held: None }
    }
}

impl<Id: PartialEq + Copy> Repeat<Id> {
    pub fn press(&mut self, id: Id, key: Key, now: Instant) {
        let repeats = matches!(
            key,
            Key::Up | Key::Down | Key::Left | Key::Right | Key::PageUp | Key::PageDown | Key::Backspace | Key::Delete | Key::BrightnessDown | Key::BrightnessUp | Key::Char(_)
        );
        self.held = repeats.then_some((id, key, now + REPEAT_AFTER));
    }

    pub fn release(&mut self, id: Id) {
        if self.held.is_some_and(|(held, ..)| held == id) {
            self.held = None;
        }
    }

    /// The repeat due by `now`, if one is: one at a time, never a burst.
    pub fn due(&mut self, now: Instant) -> Option<Key> {
        let (_, key, next) = self.held.as_mut()?;
        if *next > now {
            return None;
        }
        *next = (*next + REPEAT_EVERY).max(now + REPEAT_EVERY / 2);
        Some(*key)
    }

    /// When the next repeat is due, for a wait to end in time for it.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn next(&self) -> Option<Instant> {
        self.held.map(|(_, _, next)| next)
    }
}

/// A key code from an evdev keyboard, as a [`Key`], with the modifiers held.
/// `own`: it came from the handheld's keyboard (its Sym layer). `None` for
/// releases, modifiers and keys with no meaning here.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn translate(code: u16, shift: bool, ctrl: bool, own: bool) -> Option<Key> {
    const ROWS: [(u16, &str, &str); 4] = [
        (2, "1234567890-=", "!@#$%^&*()_+"),
        (16, "qwertyuiop[]", "QWERTYUIOP{}"),
        (30, "asdfghjkl;'`", "ASDFGHJKL:\"~"),
        (44, "zxcvbnm,./", "ZXCVBNM<>?"),
    ];
    if own {
        if let Some(ch) = sym_char(code) {
            return Some(Key::Char(ch));
        }
    }
    if let Some(key) = fn_key(code) {
        return Some(key);
    }
    Some(match code {
        1 => Key::Back,
        14 => Key::Backspace,
        15 => Key::Tab,
        28 | 96 => Key::Enter,
        57 => Key::Char(' '),
        103 => Key::Up,
        108 => Key::Down,
        105 => Key::Left,
        106 => Key::Right,
        104 => Key::PageUp,
        109 => Key::PageDown,
        _ => {
            let (base, plain, shifted) = ROWS.iter().find(|(base, row, _)| code >= *base && ((code - base) as usize) < row.len())?;
            let row = if shift { shifted } else { plain };
            let ch = row.as_bytes()[(code - base) as usize] as char;
            if ctrl && ch.is_ascii_alphabetic() {
                Key::Ctrl(ch.to_ascii_lowercase())
            } else {
                Key::Char(ch)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_read_as_the_shell_reads_them() {
        assert_eq!(Key::Char('f').plain(), Key::Up);
        assert_eq!(Key::Char('c').plain(), Key::Right);
        assert_eq!(Key::Backspace.plain(), Key::Back);
        assert_eq!(Key::Char('q').plain(), Key::Char('q'));
        assert!(Key::Ctrl('q').leaves() && Key::Leave.leaves() && !Key::Back.leaves());
        assert_eq!(translate(16, false, false, true), Some(Key::Char('q')));
        assert_eq!(translate(16, true, false, true), Some(Key::Char('Q')));
        assert_eq!(translate(16, false, true, true), Some(Key::Ctrl('q')));
        // Sym + 1 on the handheld's own keyboard; a USB keyboard's [ stays [.
        assert_eq!(translate(26, false, false, true), Some(Key::Char('!')));
        assert_eq!(translate(26, false, false, false), Some(Key::Char('[')));
        assert_eq!(translate(59, false, false, true), Some(Key::F(1)));
        assert_eq!(Key::Char('7').digit(), Some(7));
    }

    #[test]
    fn a_held_key_repeats_on_the_shells_clock() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut held = Repeat::default();
        held.press(1u16, Key::Down, t0);
        assert_eq!(held.due(at(399)), None);
        assert_eq!(held.due(at(400)), Some(Key::Down));
        assert_eq!(held.due(at(475)), Some(Key::Down));
        assert_eq!(held.due(at(1000)), Some(Key::Down));
        assert_eq!(held.due(at(1001)), None, "one at a time after a stall");
        held.release(1);
        assert_eq!(held.due(at(5000)), None);
        held.press(2, Key::Enter, t0);
        assert_eq!(held.next(), None, "Enter counts once a press");
    }
}
