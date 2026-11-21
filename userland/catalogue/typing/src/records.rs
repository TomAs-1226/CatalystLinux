//! What Typing keeps in its own folder: the best words a minute for each
//! round and word list, and the round and list last played.

use std::path::{Path, PathBuf};

use catalyst_kit::Saved;

use crate::model::Round;
use crate::words::Pack;

/// The file's name in the app's folder.
pub const FILE: &str = "records";

pub struct Records {
    saved: Saved,
}

/// What offering a finished round's speed did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Offer {
    /// It beat the best (the first round with any speed always does).
    pub new_best: bool,
    /// The best now.
    pub best: u32,
    /// The new best could not be written.
    pub unsaved: bool,
}

fn best_key(round: Round, pack: Pack) -> String {
    format!("best.{}.{}", round.key(), pack.key())
}

impl Records {
    /// The records in `dir` (the app's folder).
    pub fn in_dir(dir: &Path) -> Records {
        Records::at(dir.join(FILE))
    }

    pub fn at(path: PathBuf) -> Records {
        Records { saved: Saved::at(path) }
    }

    /// The best words a minute for a round and list; 0 when none is kept.
    pub fn best(&self, round: Round, pack: Pack) -> u32 {
        self.saved.number::<u32>(&best_key(round, pack)).unwrap_or(0).min(999)
    }

    /// A finished round's speed: kept if it beats the best.
    pub fn offer(&mut self, round: Round, pack: Pack, wpm: u32) -> Offer {
        let before = self.best(round, pack);
        if wpm <= before {
            return Offer { new_best: false, best: before, unsaved: false };
        }
        self.saved.set(&best_key(round, pack), wpm);
        Offer { new_best: true, best: wpm, unsaved: self.saved.save().is_err() }
    }

    /// The round and list last chosen (the first of each when none is).
    pub fn choice(&self) -> (Round, Pack) {
        let round = self.saved.get("round").and_then(Round::from_key).unwrap_or(Round::Sprint30);
        let pack = self.saved.get("pack").and_then(Pack::from_key).unwrap_or(Pack::English);
        (round, pack)
    }

    /// Keep the round and list chosen, for next time. Best effort: a choice
    /// that could not be written is only a choice to make again.
    pub fn choose(&mut self, round: Round, pack: Pack) {
        if self.choice() != (round, pack) {
            self.saved.set("round", round.key());
            self.saved.set("pack", pack.key());
            self.saved.save().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_best_is_kept_for_each_round_and_list() {
        let dir = std::env::temp_dir().join(format!("catalyst-typing-records-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let mut records = Records::in_dir(&dir);
        assert_eq!(records.best(Round::Sprint30, Pack::Pit), 0);
        assert_eq!(records.choice(), (Round::Sprint30, Pack::English));
        // A round with no speed is no best.
        assert_eq!(records.offer(Round::Sprint30, Pack::Pit, 0), Offer { new_best: false, best: 0, unsaved: false });
        // The first speed is a new best; a slower one is not; a faster one is.
        assert!(records.offer(Round::Sprint30, Pack::Pit, 41).new_best);
        assert_eq!(records.offer(Round::Sprint30, Pack::Pit, 41), Offer { new_best: false, best: 41, unsaved: false });
        assert_eq!(records.offer(Round::Sprint30, Pack::Pit, 38).best, 41);
        assert_eq!(records.offer(Round::Sprint30, Pack::Pit, 52), Offer { new_best: true, best: 52, unsaved: false });
        // Kept apart by round and by list, and read back from the file.
        records.offer(Round::Words10, Pack::Code, 33);
        records.choose(Round::Words10, Pack::Code);
        let back = Records::in_dir(&dir);
        assert_eq!(back.best(Round::Sprint30, Pack::Pit), 52);
        assert_eq!(back.best(Round::Words10, Pack::Code), 33);
        assert_eq!(back.best(Round::Sprint60, Pack::Pit), 0);
        assert_eq!(back.best(Round::Sprint30, Pack::English), 0);
        assert_eq!(back.choice(), (Round::Words10, Pack::Code));
        std::fs::remove_dir_all(&dir).ok();
    }
}
