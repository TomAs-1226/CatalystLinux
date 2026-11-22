//! Games won and lost at each level, and the level last played, kept in the
//! app's own folder (`record`, `key = value` lines).

use catalyst_kit::Saved;

use crate::ai::Level;

pub struct Record {
    saved: Saved,
}

impl Record {
    pub fn open() -> Record {
        Record { saved: Saved::open("record") }
    }

    /// Won and lost at `level`.
    pub fn of(&self, level: Level) -> (u32, u32) {
        let count = |what: &str| self.saved.number(&format!("{}.{what}", level.key())).unwrap_or(0);
        (count("won"), count("lost"))
    }

    /// One more game at `level`, kept at once.
    pub fn add(&mut self, level: Level, won: bool) -> std::io::Result<()> {
        let (w, l) = self.of(level);
        let key = format!("{}.{}", level.key(), if won { "won" } else { "lost" });
        self.saved.set(&key, if won { w + 1 } else { l + 1 });
        self.saved.save()
    }

    pub fn last_level(&self) -> Level {
        self.saved.get("level").and_then(Level::from_key).unwrap_or(Level::Normal)
    }

    pub fn set_last_level(&mut self, level: Level) -> std::io::Result<()> {
        if self.last_level() == level && self.saved.get("level").is_some() {
            return Ok(());
        }
        self.saved.set("level", level.key());
        self.saved.save()
    }

    /// The record at `level` in a few words.
    pub fn words(&self, level: Level) -> String {
        match self.of(level) {
            (0, 0) => "no games yet".to_owned(),
            (won, lost) => format!("won {won} · lost {lost}"),
        }
    }
}
