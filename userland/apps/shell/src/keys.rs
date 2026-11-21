//! Keys for online services: The Blue Alliance and FRC Nexus for the event
//! schedule, OpenAI and Claude for the assistant. Kept in one file beside the
//! settings on the data partition, readable only by root, and never shown
//! again in full.
//!
//! They are set during first-boot setup (on the handheld or from a computer),
//! in Settings, or by reading a file someone copied over: the same
//! `name=value` lines this file holds.

use std::path::{Path, PathBuf};

/// The services a key can be held for: (name in the file, what to call it).
pub const SERVICES: [(&str, &str); 5] = [("tba", "The Blue Alliance"), ("nexus", "FRC Nexus"), ("openai", "OpenAI"), ("claude", "Claude"), ("github", "GitHub")];

/// How many of them the handheld's own setup asks for: its Keys page has room
/// for four rows. GitHub's key comes later (Git › Account, Settings › Keys, or
/// the page for setting up from a computer).
pub const SETUP_SERVICES: usize = 4;

/// Where a key held under an older name now lives: 0.4.0 and before had one
/// "ai" key, which goes to whichever service it belongs to (Anthropic's keys
/// begin `sk-ant-`).
fn current_name<'a>(name: &'a str, value: &str) -> &'a str {
    match name {
        "ai" if value.starts_with("sk-ant-") => "claude",
        "ai" => "openai",
        other => other,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keys {
    entries: Vec<(String, String)>,
}

/// The file the keys are kept in. Search watches it change, to say which
/// keys are set; their values never leave this module.
pub fn path() -> PathBuf {
    crate::config_dir().join("keys")
}

impl Keys {
    pub fn load() -> Self {
        Self::read(&path()).unwrap_or_default()
    }

    /// Keys from a file of `name=value` lines; lines it does not know are
    /// passed over, so a team's own notes can sit in the same file.
    pub fn read(file: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(file).ok()?;
        Some(Self::parse(&text))
    }

    pub fn parse(text: &str) -> Self {
        let mut keys = Keys::default();
        for line in text.lines() {
            if let Some((name, value)) = line.split_once('=') {
                let value = value.trim();
                let name = current_name(name.trim(), value);
                if SERVICES.iter().any(|(known, _)| *known == name) && !value.is_empty() {
                    keys.set(name, value);
                }
            }
        }
        keys
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, name: &str, value: &str) {
        let value = value.trim();
        self.entries.retain(|(n, _)| n != name);
        if !value.is_empty() {
            self.entries.push((name.to_owned(), value.to_owned()));
        }
    }

    /// Take every key `other` holds.
    pub fn merge(&mut self, other: &Keys) {
        for (name, value) in &other.entries {
            self.set(name, value);
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let text: String = self.entries.iter().map(|(n, v)| format!("{n}={v}\n")).collect();
        let path = path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::sys::write_private(&path, text.as_bytes())
    }

    /// How a key is shown once set: enough to recognise it, not enough to use it.
    pub fn shown(&self, name: &str) -> String {
        match self.get(name) {
            None => "not set".into(),
            Some(value) => {
                let tail: String = value.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
                format!("set · …{tail}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_read_from_lines_and_shown_only_in_part() {
        let keys = Keys::parse("# team keys\ntba = abcdef123456\nnexus=\nsomething=else\nopenai=sk-proj-xyz-9876\n");
        assert_eq!(keys.get("tba"), Some("abcdef123456"));
        assert_eq!(keys.get("nexus"), None, "an empty value is no key");
        assert_eq!(keys.get("something"), None, "unknown names are passed over");
        assert_eq!(keys.shown("openai"), "set · …9876");
        assert_eq!(keys.shown("nexus"), "not set");
        let mut mine = Keys::parse("tba=old\nnexus=n1");
        mine.merge(&keys);
        assert_eq!((mine.get("tba"), mine.get("nexus"), mine.get("openai")), (Some("abcdef123456"), Some("n1"), Some("sk-proj-xyz-9876")));
    }

    #[test]
    fn the_old_single_ai_key_goes_to_its_own_service() {
        let claude = Keys::parse("ai=sk-ant-api03-abcd\n");
        assert_eq!((claude.get("claude"), claude.get("openai"), claude.get("ai")), (Some("sk-ant-api03-abcd"), None, None));
        let openai = Keys::parse("ai=sk-proj-1234\n");
        assert_eq!((openai.get("openai"), openai.get("claude")), (Some("sk-proj-1234"), None));
    }
}
