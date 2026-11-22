//! The three word lists, 64 words each. English and code are key-dash's own,
//! word for word; the pit list is an FRC team's: what gets said between
//! matches with the robot on the cart.

/// Which words a round deals.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pack {
    English,
    Code,
    Pit,
}

impl Pack {
    pub const ALL: [Pack; 3] = [Pack::English, Pack::Code, Pack::Pit];

    /// The name kept in the records file, and shown (lowercase, as a label).
    pub fn key(self) -> &'static str {
        match self {
            Pack::English => "english",
            Pack::Code => "code",
            Pack::Pit => "pit",
        }
    }

    pub fn from_key(key: &str) -> Option<Pack> {
        Pack::ALL.into_iter().find(|p| p.key() == key.trim())
    }

    /// The Material Symbol beside its name.
    pub fn icon(self) -> &'static str {
        match self {
            Pack::English => "language",
            Pack::Code => "terminal",
            Pack::Pit => "precision_manufacturing",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn words(self) -> &'static [&'static str; 64] {
        match self {
            Pack::English => &ENGLISH,
            Pack::Code => &CODE,
            Pack::Pit => &PIT,
        }
    }
}

/// key-dash's English list, as it ships.
pub const ENGLISH: [&str; 64] = [
    "tiny", "pixel", "cloud", "river", "light", "dream", "green", "music", //
    "paper", "water", "moon", "spark", "happy", "orbit", "quiet", "plant", //
    "sunny", "apple", "world", "night", "field", "bloom", "magic", "ocean", //
    "stone", "copper", "garden", "window", "planet", "pocket", "forest", "silver", //
    "little", "coffee", "yellow", "orange", "button", "ripple", "comet", "hello", //
    "summer", "winter", "spring", "autumn", "breeze", "thread", "smile", "toast", //
    "candle", "velvet", "feather", "meadow", "shell", "raven", "maple", "coral", //
    "tiger", "pearl", "lemon", "mint", "ember", "violet", "rain", "moss",
];

/// key-dash's code list, as it ships.
pub const CODE: [&str; 64] = [
    "array", "return", "while", "const", "float", "class", "string", "vector", //
    "input", "output", "buffer", "stack", "queue", "shell", "linux", "kernel", //
    "debug", "build", "commit", "branch", "merge", "push", "fetch", "clone", //
    "binary", "source", "header", "import", "export", "module", "public", "static", //
    "struct", "switch", "case", "break", "char", "value", "object", "method", //
    "thread", "signal", "socket", "server", "client", "packet", "screen", "render", //
    "event", "timer", "memory", "cache", "query", "index", "table", "format", //
    "parse", "token", "scope", "async", "await", "yield", "false", "true",
];

/// The pit: drivetrain, mechanisms, electrical, tools and the event itself.
pub const PIT: [&str; 64] = [
    "swerve", "intake", "elevator", "shooter", "gyro", "encoder", "brownout", "battery", //
    "bumper", "chassis", "climber", "auton", "teleop", "alliance", "match", "queue", //
    "gearbox", "sprocket", "chain", "pulley", "bearing", "shaft", "motor", "falcon", //
    "kraken", "breaker", "radio", "camera", "apriltag", "piston", "solenoid", "pressure", //
    "wrench", "rivet", "drill", "ziptie", "bracket", "gusset", "polycarb", "wheel", //
    "tread", "roller", "hopper", "indexer", "turret", "wrist", "pivot", "winch", //
    "ratchet", "scout", "driver", "operator", "coach", "mentor", "rookie", "playoff", //
    "penalty", "endgame", "roborio", "odometry", "joystick", "banner", "crimp", "ferrule",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_list_is_64_different_lowercase_words() {
        for pack in Pack::ALL {
            let words = pack.words();
            assert_eq!(words.len(), 64);
            for (i, w) in words.iter().enumerate() {
                assert!(!w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase()), "{pack:?}: {w:?}");
                assert!(!words[..i].contains(w), "{pack:?} has {w:?} twice");
            }
            assert_eq!(Pack::from_key(pack.key()), Some(pack));
        }
        // The pit list's own rule: 4 to 8 letters.
        assert!(PIT.iter().all(|w| (4..=8).contains(&w.len())), "{:?}", PIT.iter().find(|w| !(4..=8).contains(&w.len())));
    }

    #[test]
    fn the_originals_are_kept_as_they_ship() {
        // Spot checks against key-dash's typing_model.cpp: first, middle, last.
        assert_eq!((ENGLISH[0], ENGLISH[31], ENGLISH[63]), ("tiny", "silver", "moss"));
        assert_eq!((CODE[0], CODE[31], CODE[63]), ("array", "static", "true"));
    }
}
