//! What the vault keeps: entries of four kinds, and the body they are
//! sealed as.
//!
//! Every field of an entry is wiped from memory when the entry is let go
//! (its copies too), and none of them is ever printed: `{:?}` shows the kind
//! and the title alone.
//!
//! The body, before it is sealed: a version byte (1), the number of entries,
//! and each entry as its length and then its fields, each a tag byte, a
//! length and the bytes. A field this version does not know is passed
//! over, so a newer vault's entries still open here.

use zeroize::{Zeroize, Zeroizing};

/// What an entry is: the same fields, named for what each holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Login,
    Note,
    Wifi,
    Pin,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Login, Kind::Note, Kind::Wifi, Kind::Pin];

    fn byte(self) -> u8 {
        match self {
            Kind::Login => 0,
            Kind::Note => 1,
            Kind::Wifi => 2,
            Kind::Pin => 3,
        }
    }

    fn from_byte(b: u8) -> Kind {
        match b {
            1 => Kind::Note,
            2 => Kind::Wifi,
            3 => Kind::Pin,
            _ => Kind::Login,
        }
    }

    /// What it is called in a list ("login", "wi-fi").
    pub fn word(self) -> &'static str {
        match self {
            Kind::Login => "login",
            Kind::Note => "secure note",
            Kind::Wifi => "wi-fi",
            Kind::Pin => "card pin",
        }
    }

    /// Its name as a page's title.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Login => "Login",
            Kind::Note => "Note",
            Kind::Wifi => "Wi-Fi",
            Kind::Pin => "Card PIN",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Login => "password",
            Kind::Note => "sticky_note_2",
            Kind::Wifi => "wifi",
            Kind::Pin => "pin",
        }
    }

    pub fn step(self, step: i32) -> Kind {
        let at = Kind::ALL.iter().position(|k| *k == self).unwrap_or(0) as i32;
        Kind::ALL[(at + step).rem_euclid(Kind::ALL.len() as i32) as usize]
    }

    /// The fields this kind has, in the order they show.
    pub fn fields(self) -> &'static [Field] {
        match self {
            Kind::Login => &[Field::Title, Field::Username, Field::Password, Field::Totp, Field::Url, Field::Notes],
            Kind::Note => &[Field::Title, Field::Notes],
            Kind::Wifi => &[Field::Title, Field::Password, Field::Notes],
            Kind::Pin => &[Field::Title, Field::Username, Field::Password, Field::Notes],
        }
    }

    /// What a field is called on an entry of this kind.
    pub fn label(self, field: Field) -> &'static str {
        match (self, field) {
            (Kind::Wifi, Field::Title) => "network",
            (Kind::Pin, Field::Title) => "card",
            (_, Field::Title) => "title",
            (Kind::Pin, Field::Username) => "last digits",
            (_, Field::Username) => "username",
            (Kind::Pin, Field::Password) => "pin",
            (_, Field::Password) => "password",
            (_, Field::Url) => "website",
            (Kind::Note, Field::Notes) => "note",
            (_, Field::Notes) => "notes",
            (_, Field::Totp) => "2fa code",
        }
    }
}

/// An entry's fields, as the editor and the page name them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Title,
    Username,
    Password,
    Url,
    Notes,
    Totp,
}

impl Field {
    /// The longest it may be typed or imported.
    pub fn limit(self) -> usize {
        match self {
            Field::Notes => 4000,
            Field::Url | Field::Totp => 600,
            _ => 200,
        }
    }
}

/// One thing kept in the vault.
#[derive(Clone, PartialEq)]
pub struct Entry {
    /// Its name in the vault, random, the same for its life: Spotlight
    /// opens it by this, and never knows anything else of it.
    pub id: u64,
    pub kind: Kind,
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub notes: String,
    /// Its two-factor secret, as an `otpauth://totp/` address; empty for none.
    pub totp: String,
    /// Unix seconds.
    pub created: i64,
    pub modified: i64,
}

impl Drop for Entry {
    fn drop(&mut self) {
        for s in [&mut self.title, &mut self.username, &mut self.password, &mut self.url, &mut self.notes, &mut self.totp] {
            s.zeroize();
        }
    }
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Entry({:?}, {} fields, secrets withheld)", self.kind, self.kind.fields().len())
    }
}

impl Entry {
    pub fn new(kind: Kind, id: u64, now: i64) -> Entry {
        Entry { id, kind, title: String::new(), username: String::new(), password: String::new(), url: String::new(), notes: String::new(), totp: String::new(), created: now, modified: now }
    }

    pub fn get(&self, field: Field) -> &str {
        match field {
            Field::Title => &self.title,
            Field::Username => &self.username,
            Field::Password => &self.password,
            Field::Url => &self.url,
            Field::Notes => &self.notes,
            Field::Totp => &self.totp,
        }
    }

    pub fn get_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Title => &mut self.title,
            Field::Username => &mut self.username,
            Field::Password => &mut self.password,
            Field::Url => &mut self.url,
            Field::Notes => &mut self.notes,
            Field::Totp => &mut self.totp,
        }
    }

    /// Set a field, wiping what it held.
    pub fn set(&mut self, field: Field, value: &str) {
        let s = self.get_mut(field);
        s.zeroize();
        s.push_str(value);
    }

    /// Its title, or what it is when it has none.
    pub fn name(&self) -> &str {
        if self.title.trim().is_empty() {
            match self.kind {
                Kind::Login => "Untitled login",
                Kind::Note => "Untitled note",
                Kind::Wifi => "Unnamed network",
                Kind::Pin => "Unnamed card",
            }
        } else {
            self.title.trim()
        }
    }

    /// What a list shows beside its name: the username, or what it is.
    pub fn summary(&self) -> String {
        match self.kind {
            Kind::Login if !self.username.is_empty() => self.username.clone(),
            Kind::Pin if !self.username.is_empty() => format!("card ··{}", self.username.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>()),
            kind => kind.word().to_owned(),
        }
    }

    /// Found by a search: its title or username, any case.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        q.is_empty() || self.title.to_lowercase().contains(&q) || self.username.to_lowercase().contains(&q)
    }

    /// The same login, password and all (an import that is already here).
    pub fn same_as(&self, other: &Entry) -> bool {
        self.kind == other.kind && self.title == other.title && self.username == other.username && self.password == other.password && self.notes == other.notes
    }
}

/// A new entry's id: random, never 0.
pub fn new_id() -> u64 {
    let mut b = [0u8; 8];
    loop {
        if super::crypto::random(&mut b).is_err() {
            // No random numbers: the time is unique enough for a name.
            return std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64 | 1);
        }
        let id = u64::from_le_bytes(b);
        if id != 0 {
            return id;
        }
    }
}

/// Entries in the order a list shows them: by name, any case.
pub fn sort(entries: &mut [Entry]) {
    entries.sort_by_cached_key(|e| (e.name().to_lowercase(), e.id));
}

// ------------------------------------------------------------- the body ---

const FORMAT: u8 = 1;

const T_ID: u8 = 1;
const T_KIND: u8 = 2;
const T_TITLE: u8 = 3;
const T_USERNAME: u8 = 4;
const T_PASSWORD: u8 = 5;
const T_URL: u8 = 6;
const T_NOTES: u8 = 7;
const T_TOTP: u8 = 8;
const T_CREATED: u8 = 9;
const T_MODIFIED: u8 = 10;

fn strings(e: &Entry) -> [(u8, &str); 6] {
    [(T_TITLE, &e.title), (T_USERNAME, &e.username), (T_PASSWORD, &e.password), (T_URL, &e.url), (T_NOTES, &e.notes), (T_TOTP, &e.totp)]
}

fn entry_len(e: &Entry) -> usize {
    (5 + 8) + (5 + 1) + 2 * (5 + 8) + strings(e).iter().map(|(_, s)| 5 + s.len()).sum::<usize>()
}

/// The entries as the body's bytes, in one buffer sized beforehand (so it
/// is never grown, and never leaves a copy behind), wiped when let go.
pub fn encode(entries: &[Entry]) -> Zeroizing<Vec<u8>> {
    let total = 1 + 4 + entries.iter().map(|e| 4 + entry_len(e)).sum::<usize>();
    let mut out = Zeroizing::new(Vec::with_capacity(total));
    out.push(FORMAT);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    let field = |out: &mut Vec<u8>, tag: u8, bytes: &[u8]| {
        out.push(tag);
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    };
    for e in entries {
        out.extend_from_slice(&(entry_len(e) as u32).to_le_bytes());
        field(&mut out, T_ID, &e.id.to_le_bytes());
        field(&mut out, T_KIND, &[e.kind.byte()]);
        field(&mut out, T_CREATED, &e.created.to_le_bytes());
        field(&mut out, T_MODIFIED, &e.modified.to_le_bytes());
        for (tag, s) in strings(e) {
            field(&mut out, tag, s.as_bytes());
        }
    }
    debug_assert_eq!(out.len(), total);
    debug_assert_eq!(out.capacity(), total);
    out
}

/// The body's bytes as entries; `None` if they are not whole (which a body
/// that opened never is, short of a fault in the writing).
pub fn decode(bytes: &[u8]) -> Option<Vec<Entry>> {
    let mut at = 0usize;
    let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
        let s = bytes.get(*at..at.checked_add(n)?)?;
        *at += n;
        Some(s)
    };
    let format = *take(&mut at, 1)?.first()?;
    if format == 0 || format > FORMAT {
        return None;
    }
    let count = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?) as usize;
    let mut out = Vec::with_capacity(count.min(10_000));
    for _ in 0..count {
        let len = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?) as usize;
        let end = at.checked_add(len)?;
        if end > bytes.len() {
            return None;
        }
        let mut e = Entry::new(Kind::Login, 0, 0);
        while at < end {
            let tag = take(&mut at, 1)?[0];
            let n = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?) as usize;
            let v = take(&mut at, n)?;
            if at > end {
                return None;
            }
            let num = || v.try_into().ok().map(i64::from_le_bytes);
            match tag {
                T_ID => e.id = u64::from_le_bytes(v.try_into().ok()?),
                T_KIND => e.kind = Kind::from_byte(*v.first()?),
                T_CREATED => e.created = num()?,
                T_MODIFIED => e.modified = num()?,
                T_TITLE | T_USERNAME | T_PASSWORD | T_URL | T_NOTES | T_TOTP => {
                    let s = std::str::from_utf8(v).ok()?;
                    let field = match tag {
                        T_TITLE => Field::Title,
                        T_USERNAME => Field::Username,
                        T_PASSWORD => Field::Password,
                        T_URL => Field::Url,
                        T_NOTES => Field::Notes,
                        _ => Field::Totp,
                    };
                    e.set(field, s);
                }
                // A field from a newer version: passed over.
                _ => {}
            }
        }
        if e.id == 0 {
            e.id = new_id();
        }
        out.push(e);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn demo() -> Vec<Entry> {
        let mut a = Entry::new(Kind::Login, 11, 1_700_000_000);
        a.set(Field::Title, "GitHub");
        a.set(Field::Username, "pit-crew@example.org");
        a.set(Field::Password, "Tr0ub4dor&3-demo");
        a.set(Field::Url, "https://github.com");
        a.set(Field::Totp, "otpauth://totp/GitHub:pit-crew?secret=JBSWY3DPEHPK3PXP&issuer=GitHub");
        let mut b = Entry::new(Kind::Wifi, 12, 1_700_000_100);
        b.set(Field::Title, "Pit Wi-Fi");
        b.set(Field::Password, "élévateur-5805");
        let mut c = Entry::new(Kind::Note, 13, 1_700_000_200);
        c.set(Field::Title, "Locker");
        c.set(Field::Notes, "combination 12-34-56\nsecond line");
        vec![a, b, c]
    }

    #[test]
    fn entries_go_through_the_body_whole() {
        let entries = demo();
        let bytes = encode(&entries);
        let back = decode(&bytes).unwrap();
        assert_eq!(back, entries);
        assert_eq!(decode(&encode(&[])).unwrap(), Vec::<Entry>::new());
        // Cut anywhere, it does not read as entries.
        for cut in 0..bytes.len() {
            let got = decode(&bytes[..cut]);
            assert!(got.is_none(), "cut at {cut} read as {got:?}");
        }
    }

    #[test]
    fn a_field_from_a_newer_vault_is_passed_over() {
        let mut bytes = encode(&demo()[..1]).to_vec();
        // An unknown field (tag 99) added to the first entry.
        let extra = [99u8, 3, 0, 0, 0, b'n', b'e', b'w'];
        let len = u32::from_le_bytes(bytes[5..9].try_into().unwrap()) as usize + extra.len();
        bytes[5..9].copy_from_slice(&(len as u32).to_le_bytes());
        bytes.extend_from_slice(&extra);
        assert_eq!(decode(&bytes).unwrap(), demo()[..1].to_vec());
    }

    #[test]
    fn an_entry_never_prints_its_secrets() {
        for e in demo() {
            let shown = format!("{e:?} {:?}", vec![e.clone()]);
            for secret in ["Tr0ub4dor", "élévateur", "12-34-56", "JBSWY3DP", "pit-crew"] {
                assert!(!shown.contains(secret), "{shown}");
            }
        }
    }

    #[test]
    fn kinds_name_their_fields_and_lists_say_what_each_is() {
        assert_eq!(Kind::Pin.label(Field::Password), "pin");
        assert_eq!(Kind::Wifi.label(Field::Title), "network");
        assert_eq!(Kind::Note.fields(), [Field::Title, Field::Notes]);
        let mut pin = Entry::new(Kind::Pin, 1, 0);
        pin.set(Field::Username, "4111 1111 1111 4821");
        assert_eq!(pin.summary(), "card ··4821");
        assert_eq!(demo()[0].summary(), "pit-crew@example.org");
        assert_eq!(demo()[1].summary(), "wi-fi");
        assert!(demo()[0].matches("GITHUB") && demo()[0].matches("pit-crew") && !demo()[0].matches("Tr0ub"), "never by its password");
        assert_eq!(Kind::Login.step(-1), Kind::Pin);
    }
}
