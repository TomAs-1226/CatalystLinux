//! Bringing passwords in, and taking a backup out.
//!
//! - **CSV exports** from Bitwarden, 1Password and KeePass (KeePassXC's and
//!   KeePass 2's), told apart by their columns, the common ones read: the
//!   title, username, password, website, notes and two-factor secret. A
//!   Bitwarden secure note comes in as a note. Any other CSV with columns
//!   named so (Chrome's, Firefox's) comes in the same way.
//! - **`otpauth://` addresses**, one to a line in a text file (as Aegis and
//!   other authenticators export them): each a login with its code.
//! - **Backups** this vault made (`.cvault`), opened with their password.
//!
//! The CSV is read into memory that is wiped once its rows are entries, and
//! nothing of it is written anywhere but the vault. Afterwards the vault
//! offers to shred it: overwritten, flushed to the card, renamed, then
//! deleted. A memory card moves its blocks about as it wears, so an old copy
//! may survive any overwrite: shredding is the best this side of the card
//! can do, and the vault says so.

use std::path::{Path, PathBuf};

use zeroize::{Zeroize, Zeroizing};

use super::entry::{Entry, Field, Kind};
use super::totp::Totp;

/// Where an export came from, by its columns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Bitwarden,
    OnePassword,
    KeePassXc,
    KeePass,
    Csv,
    Authenticator,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Bitwarden => "Bitwarden",
            Source::OnePassword => "1Password",
            Source::KeePassXc => "KeePassXC",
            Source::KeePass => "KeePass",
            Source::Csv => "CSV",
            Source::Authenticator => "2FA codes",
        }
    }
}

/// What reading an export found: its entries, and what could not be read.
pub struct Found {
    pub source: Source,
    pub entries: Vec<Entry>,
    /// Rows with nothing to keep (no title, username, password or notes).
    pub empty: usize,
    /// Two-factor secrets that were not understood (left out).
    pub bad_codes: usize,
}

/// RFC 4180 CSV: quoted fields with commas, quotes ("") and line breaks
/// inside, CRLF or LF, a UTF-8 byte-order mark passed over.
pub fn csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                c => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                any = false;
            }
            c => field.push(c),
        }
    }
    if any || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| !(r.len() == 1 && r[0].trim().is_empty()));
    rows
}

/// The column a name stands for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Col {
    Field(Field),
    Type,
    Ignore,
}

fn column(name: &str) -> Col {
    match name.trim().to_lowercase().as_str() {
        "title" | "name" | "account" | "item name" => Col::Field(Field::Title),
        "username" | "login_username" | "login name" | "user name" | "login" | "user" | "email" => Col::Field(Field::Username),
        "password" | "login_password" | "pass" => Col::Field(Field::Password),
        "url" | "login_uri" | "website" | "web site" | "uri" | "urls" => Col::Field(Field::Url),
        "notes" | "note" | "comments" | "extra" => Col::Field(Field::Notes),
        "totp" | "login_totp" | "otpauth" | "otp" | "one-time password" | "2fa" => Col::Field(Field::Totp),
        "type" => Col::Type,
        _ => Col::Ignore,
    }
}

fn source_of(header: &[String]) -> Source {
    let has = |n: &str| header.iter().any(|h| h.trim().eq_ignore_ascii_case(n));
    if has("login_password") || has("login_uri") {
        Source::Bitwarden
    } else if has("otpauth") || (has("archived") && has("favorite")) {
        Source::OnePassword
    } else if has("group") && has("title") && has("totp") {
        Source::KeePassXc
    } else if has("login name") || (has("account") && has("web site")) {
        Source::KeePass
    } else {
        Source::Csv
    }
}

/// The entries in a CSV export; `None` if it is not one (no column for a
/// title, username or password).
pub fn read_csv(text: &str, now: i64) -> Option<Found> {
    let mut rows = csv(text).into_iter();
    let header = rows.next()?;
    let cols: Vec<Col> = header.iter().map(|h| column(h)).collect();
    let wanted = [Field::Title, Field::Username, Field::Password];
    if !cols.iter().any(|c| matches!(c, Col::Field(f) if wanted.contains(f))) {
        return None;
    }
    let mut found = Found { source: source_of(&header), entries: Vec::new(), empty: 0, bad_codes: 0 };
    for mut row in rows {
        let mut e = Entry::new(Kind::Login, super::entry::new_id(), now);
        let mut note_type = false;
        for (col, value) in cols.iter().zip(row.iter()) {
            let value = value.trim();
            match col {
                Col::Field(Field::Totp) if !value.is_empty() => match Totp::parse(value) {
                    Ok(t) => e.set(Field::Totp, &t.uri()),
                    Err(_) => found.bad_codes += 1,
                },
                Col::Field(f) => {
                    let limit = f.limit();
                    let cut: String = value.chars().take(limit).collect();
                    let now = e.get(*f);
                    // A second column for the same field (KeePass's "URL"
                    // and "Web Site") fills it only if it is empty.
                    if now.is_empty() {
                        e.set(*f, &cut);
                    }
                }
                Col::Type => note_type = value.eq_ignore_ascii_case("note") || value.eq_ignore_ascii_case("securenote"),
                Col::Ignore => {}
            }
        }
        for v in &mut row {
            v.zeroize();
        }
        if e.title.is_empty() && e.username.is_empty() && e.password.is_empty() && e.notes.is_empty() {
            found.empty += 1;
            continue;
        }
        if note_type || (e.username.is_empty() && e.password.is_empty() && e.url.is_empty() && !e.notes.is_empty()) {
            e.kind = Kind::Note;
        }
        if e.title.is_empty() {
            let site = e.url.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or("").to_owned();
            let title = if site.is_empty() { e.username.clone() } else { site };
            e.set(Field::Title, &title);
        }
        found.entries.push(e);
    }
    Some(found)
}

/// The two-factor codes in a text of `otpauth://` addresses, one a line.
pub fn read_otpauth(text: &str, now: i64) -> Option<Found> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| l.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("otpauth:"))).collect();
    if lines.is_empty() {
        return None;
    }
    let mut found = Found { source: Source::Authenticator, entries: Vec::new(), empty: 0, bad_codes: 0 };
    for line in lines {
        match Totp::parse(line) {
            Ok(t) => found.entries.push(from_totp(&t, now)),
            Err(_) => found.bad_codes += 1,
        }
    }
    Some(found)
}

/// A login holding a two-factor code, named as its address names it.
pub fn from_totp(t: &Totp, now: i64) -> Entry {
    let mut e = Entry::new(Kind::Login, super::entry::new_id(), now);
    let title = if t.issuer.is_empty() { if t.account.is_empty() { "2FA code".to_owned() } else { t.account.clone() } } else { t.issuer.clone() };
    e.set(Field::Title, &title);
    e.set(Field::Username, &t.account);
    e.set(Field::Totp, &t.uri());
    e
}

/// An export read from a file: CSV, or `otpauth://` lines. The bytes are
/// wiped as soon as they are entries.
pub fn read_file(path: &Path, now: i64) -> Result<Found, String> {
    let bytes = Zeroizing::new(std::fs::read(path).map_err(|e| format!("could not read it: {e}"))?);
    let text = std::str::from_utf8(&bytes).map_err(|_| "not text (save it as UTF-8 CSV)".to_owned())?;
    read_otpauth(text, now).or_else(|| read_csv(text, now)).ok_or_else(|| "no passwords or 2fa codes in it".to_owned())
}

/// What a file in the storage is, for the import page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileKind {
    /// A CSV export or a text of `otpauth://` lines: plaintext.
    Export,
    /// A backup the vault made.
    Backup,
}

/// The exports and backups at the top of the storage (`/data`), newest first.
pub fn files(dir: &Path) -> Vec<(PathBuf, FileKind)> {
    let mut out: Vec<(PathBuf, FileKind, std::time::SystemTime)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?.to_lowercase();
            let kind = match ext.as_str() {
                "csv" => FileKind::Export,
                "cvault" => FileKind::Backup,
                "txt" if looks_like_otpauth(&path) => FileKind::Export,
                _ => return None,
            };
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| (path, kind, meta.modified().unwrap_or(std::time::UNIX_EPOCH)))
        })
        .collect();
    out.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    out.into_iter().map(|(p, k, _)| (p, k)).collect()
}

/// A text file whose first words are an `otpauth://` address.
fn looks_like_otpauth(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 64];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let start = String::from_utf8_lossy(&head[..n]).trim_start().to_lowercase();
    head.zeroize();
    start.starts_with("otpauth://")
}

/// Shred a file: its bytes overwritten with zeros and flushed to the card,
/// the name changed, then removed. Best effort on flash (see the module).
pub fn shred(path: &Path) -> std::io::Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let len = std::fs::metadata(path)?.len();
    {
        let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
        f.seek(SeekFrom::Start(0))?;
        let zeros = [0u8; 4096];
        let mut left = len;
        while left > 0 {
            let n = left.min(zeros.len() as u64) as usize;
            f.write_all(&zeros[..n])?;
            left -= n as u64;
        }
        f.sync_all()?;
        f.set_len(0)?;
        f.sync_all()?;
    }
    let renamed = path.with_file_name(format!(".shredded-{}", super::entry::new_id()));
    let gone = if std::fs::rename(path, &renamed).is_ok() { renamed } else { path.to_path_buf() };
    std::fs::remove_file(&gone)?;
    #[cfg(unix)]
    if let Some(dir) = gone.parent().and_then(|d| std::fs::File::open(d).ok()) {
        dir.sync_all().ok();
    }
    Ok(())
}

/// Where a new backup goes: `vault-backup-2026-11-21.cvault` at the top of
/// the storage, `-2`, `-3` after it on the same day.
pub fn backup_path(dir: &Path, day: &str) -> PathBuf {
    let mut path = dir.join(format!("vault-backup-{day}.cvault"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("vault-backup-{day}-{n}.cvault"));
        n += 1;
    }
    path
}

/// Add `found` to `entries`, leaving out what is there already: (added, already there).
pub fn merge(entries: &mut Vec<Entry>, found: Vec<Entry>) -> (usize, usize) {
    let (mut added, mut there) = (0, 0);
    for e in found {
        if entries.iter().any(|have| have.same_as(&e)) {
            there += 1;
        } else {
            entries.push(e);
            added += 1;
        }
    }
    (added, there)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BITWARDEN: &str = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
        ,,login,GitHub,,,0,https://github.com,pit-crew@example.org,Tr0ub4dor&3-demo,JBSWY3DPEHPK3PXP\n\
        Team,1,login,\"Scouting, app\",\"line one\nline two\",,0,https://scout.example.org,scout,\"quote\"\"d-demo\",\n\
        ,,note,Locker,combination 12-34-56,,0,,,,\n";
    const ONEPASSWORD: &str = "Title,Url,Username,Password,OTPAuth,Favorite,Archived,Tags,Notes\r\n\
        Pit Wi-Fi,,,frc5805-demo,,false,false,,\r\n\
        Blue Alliance,https://www.thebluealliance.com,mentor,tba-demo-pass,otpauth://totp/TBA:mentor?secret=GEZDGNBVGY3TQOJQ&issuer=TBA,false,false,,read key elsewhere\r\n";
    const KEEPASSXC: &str = "\"Group\",\"Title\",\"Username\",\"Password\",\"URL\",\"Notes\",\"TOTP\",\"Icon\",\"Last Modified\",\"Created\"\n\
        \"Root\",\"Robot laptop\",\"driver\",\"kp-demo-1\",\"\",\"\",\"\",\"0\",\"2026-01-01T00:00:00Z\",\"2026-01-01T00:00:00Z\"\n\
        \"Root\",\"\",\"\",\"\",\"\",\"\",\"\",\"0\",\"\",\"\"\n";
    const KEEPASS2: &str = "\"Account\",\"Login Name\",\"Password\",\"Web Site\",\"Comments\"\n\"FIRST\",\"team5805\",\"kp2-demo\",\"https://frcportal.example.org\",\"\"\n";

    #[test]
    fn csv_reads_quotes_commas_and_line_breaks() {
        let rows = csv("\u{feff}a,b,c\r\n\"x, y\",\"say \"\"hi\"\"\",\"two\nlines\"\n\n1,,3");
        assert_eq!(rows, vec![vec!["a", "b", "c"], vec!["x, y", "say \"hi\"", "two\nlines"], vec!["1", "", "3"]]);
        assert!(csv("").is_empty());
    }

    #[test]
    fn bitwarden_1password_and_keepass_exports_come_in() {
        let bw = read_csv(BITWARDEN, 0).unwrap();
        assert_eq!(bw.source, Source::Bitwarden);
        assert_eq!(bw.entries.len(), 3);
        let gh = &bw.entries[0];
        assert_eq!((gh.title.as_str(), gh.username.as_str(), gh.password.as_str(), gh.url.as_str()), ("GitHub", "pit-crew@example.org", "Tr0ub4dor&3-demo", "https://github.com"));
        assert!(Totp::parse(&gh.totp).is_ok(), "a bare secret becomes an address");
        assert_eq!((bw.entries[1].title.as_str(), bw.entries[1].password.as_str(), bw.entries[1].notes.as_str()), ("Scouting, app", "quote\"d-demo", "line one\nline two"));
        assert_eq!(bw.entries[2].kind, Kind::Note);

        let op = read_csv(ONEPASSWORD, 0).unwrap();
        assert_eq!(op.source, Source::OnePassword);
        assert_eq!(op.entries.len(), 2);
        assert_eq!(op.entries[1].notes, "read key elsewhere");
        assert_eq!(Totp::parse(&op.entries[1].totp).unwrap().issuer, "TBA");

        let kp = read_csv(KEEPASSXC, 0).unwrap();
        assert_eq!((kp.source, kp.entries.len(), kp.empty), (Source::KeePassXc, 1, 1));
        let kp2 = read_csv(KEEPASS2, 0).unwrap();
        assert_eq!(kp2.source, Source::KeePass);
        assert_eq!((kp2.entries[0].title.as_str(), kp2.entries[0].username.as_str(), kp2.entries[0].url.as_str()), ("FIRST", "team5805", "https://frcportal.example.org"));
        // Not an export at all.
        assert!(read_csv("date,match,score\n1,2,3\n", 0).is_none());
    }

    #[test]
    fn otpauth_lines_come_in_as_codes_and_bad_ones_are_counted() {
        let text = "otpauth://totp/GitHub:pit?secret=JBSWY3DPEHPK3PXP&issuer=GitHub\n\notpauth://totp/x?secret=!!\nOTPAUTH://totp/Discord:me?secret=GEZDGNBVGY3TQOJQ\n";
        let found = read_otpauth(text, 0).unwrap();
        assert_eq!((found.entries.len(), found.bad_codes), (2, 1));
        assert_eq!((found.entries[0].title.as_str(), found.entries[0].username.as_str()), ("GitHub", "pit"));
        assert_eq!(found.entries[1].title, "Discord");
        assert!(read_otpauth("just words", 0).is_none());
    }

    #[test]
    fn an_import_skips_what_is_already_there() {
        let mut have = read_csv(BITWARDEN, 0).unwrap().entries;
        let again = read_csv(BITWARDEN, 0).unwrap().entries;
        assert_eq!(merge(&mut have, again), (0, 3));
        let more = read_csv(ONEPASSWORD, 0).unwrap().entries;
        assert_eq!(merge(&mut have, more), (2, 0));
        assert_eq!(have.len(), 5);
    }

    #[test]
    fn a_shredded_file_is_gone_and_the_list_finds_exports_and_backups() {
        let dir = std::env::temp_dir().join(format!("catalyst-vault-import-{}-{}", std::process::id(), super::super::entry::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bitwarden.csv"), BITWARDEN).unwrap();
        std::fs::write(dir.join("codes.txt"), "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP\n").unwrap();
        std::fs::write(dir.join("pit notes.txt"), "not codes").unwrap();
        std::fs::write(dir.join("vault-backup-2026-11-21.cvault"), b"CATVAULT").unwrap();
        let mut names: Vec<String> = files(&dir).iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["bitwarden.csv", "codes.txt", "vault-backup-2026-11-21.cvault"]);
        assert_eq!(read_file(&dir.join("bitwarden.csv"), 0).unwrap().entries.len(), 3);
        shred(&dir.join("bitwarden.csv")).unwrap();
        assert!(!dir.join("bitwarden.csv").exists());
        assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".shredded")), "nothing left behind");
        assert_eq!(backup_path(&dir, "2026-11-21").file_name().unwrap(), "vault-backup-2026-11-21-2.cvault");
        std::fs::remove_dir_all(&dir).ok();
    }
}
