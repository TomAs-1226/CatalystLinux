//! Records off the handheld with no internet: as CSV for a spreadsheet, and
//! as QR codes another handheld (or a phone) reads.
//!
//! A code is plain text a phone shows as it is: a first line
//! `SCOUT1 <i>/<n> <set> <form tag> <event>`, then records as CSV rows,
//! whole rows only, so each code imports on its own and a set read in any
//! order, or read twice, adds each record once. `set` tells one export's
//! codes from another's; the form tag says which form's columns the rows hold.

use std::path::{Path, PathBuf};

use super::form::{Form, Kind};
use super::store::{Record, Station, Version};

/// What starts every code.
pub const MAGIC: &str = "SCOUT1";

/// The most a code holds, header and rows. It stays at QR version 11 or
/// under (61 modules), two pixels a module in the panel's height.
pub const CODE_BYTES: usize = 300;

/// The longest notes a code carries; the CSV file has them whole.
const NOTES_IN_CODE: usize = 120;

/// FNV-1a, as hex: a short name for some bytes.
pub fn hash(bytes: &[u8]) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in bytes {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

// --------------------------------------------------------------------- csv ---

/// A value for a CSV cell, quoted when it must be.
pub fn cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// A CSV line's cells (quotes undone).
pub fn cells(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted, mut chars) = (Vec::new(), String::new(), false, line.chars().peekable());
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (false, '"') if cur.is_empty() => quoted = true,
            (false, ',') => out.push(std::mem::take(&mut cur)),
            (_, c) => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// A number as short as it reads: 3, not 3.0.
fn short_num(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Notes on one line, cut to `most` characters.
fn one_line(s: &str, most: usize) -> String {
    s.replace(['\n', '\r'], " ").chars().take(most).collect()
}

/// The CSV file's columns before the form's: who, where, when.
const FIXED: [&str; 7] = ["event", "match", "team", "station", "scout", "updated", "by"];

/// One record as a code's row: the match's short key (the event is in the
/// code's first line), team, station, scout, when, who edited it last when
/// that was not the scout; then the values, nothing for a zero.
fn code_row(form: &Form, r: &Record) -> String {
    let tail = r.match_key.rsplit('_').next().unwrap_or(&r.match_key);
    let by = if r.by == r.scout { "" } else { r.by.as_str() };
    let mut out = vec![cell(tail), r.team.to_string(), r.station.short(), cell(&r.scout), r.updated.to_string(), cell(by)];
    out.extend(form.fields.iter().map(|f| {
        let c = f.column(form);
        match f.kind {
            Kind::Text(_) => cell(&one_line(r.texts.get(&c).map_or("", String::as_str), NOTES_IN_CODE)),
            _ if r.value(&c) == 0.0 => String::new(),
            _ => short_num(r.value(&c)),
        }
    }));
    out.join(",")
}

/// A record from a code's row, at `event`.
fn from_code_row(form: &Form, event: &str, line: &str) -> Option<Record> {
    let c = cells(line);
    if c.len() != 6 + form.fields.len() {
        return None;
    }
    let match_key = format!("{event}_{}", c[0]);
    let mut r = Record {
        event: event.to_owned(),
        label: label_of(&match_key),
        match_key,
        team: c[1].parse().ok()?,
        station: Station::parse(&c[2])?,
        form: form.id.clone(),
        scout: c[3].clone(),
        updated: c[4].parse().ok()?,
        by: if c[5].is_empty() { c[3].clone() } else { c[5].clone() },
        ..Record::default()
    };
    for (f, v) in form.fields.iter().zip(&c[6..]) {
        if v.is_empty() {
            continue;
        }
        let col = f.column(form);
        match f.kind {
            Kind::Text(_) => {
                r.texts.insert(col, v.clone());
            }
            _ => {
                r.values.insert(col, v.parse().ok()?);
            }
        }
    }
    Some(r)
}

/// "Q31" from `2026nyro_qm31`, "M2" from `…_sf2m1`, "F1" from `…_f1m1`.
pub fn label_of(match_key: &str) -> String {
    let tail = match_key.rsplit('_').next().unwrap_or(match_key);
    let (level, set, number) = super::store::match_order(tail);
    match level {
        0 => format!("Q{number}"),
        3 => format!("M{set}"),
        4 => format!("F{number}"),
        _ => tail.to_uppercase(),
    }
}

/// The CSV file: a heading row, then a row for each record with its totals.
pub fn csv(form: &Form, records: &[&Record]) -> String {
    let mut head: Vec<String> = FIXED.iter().map(|s| (*s).to_owned()).collect();
    head.extend(form.fields.iter().map(|f| f.column(form).replace('.', "_")));
    head.extend(form.totals.iter().map(|t| t.id.clone()));
    let mut out = head.join(",");
    out.push('\n');
    for r in records {
        let mut line = vec![cell(&r.event), cell(&r.match_key), r.team.to_string(), r.station.short(), cell(&r.scout), r.updated.to_string(), cell(&r.by)];
        line.extend(form.fields.iter().map(|f| {
            let c = f.column(form);
            match f.kind {
                Kind::Text(_) => cell(&one_line(r.texts.get(&c).map_or("", String::as_str), usize::MAX)),
                _ => short_num(r.value(&c)),
            }
        }));
        line.extend(form.totals.iter().map(|t| short_num(form.total(t, &r.values))));
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

/// Write an event's CSV to `<root>/export/<event>.csv`.
pub fn export_csv(root: &Path, form: &Form, event: &str, records: &[&Record]) -> std::io::Result<PathBuf> {
    let dir = root.join("export");
    std::fs::create_dir_all(&dir)?;
    let name: String = event.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let path = dir.join(format!("{}.csv", if name.is_empty() { "scouting" } else { &name }));
    crate::sys::write_atomic(&path, csv(form, records).as_bytes())?;
    Ok(path)
}

// ------------------------------------------------------------------- codes ---

/// An event's records as codes' texts, as many as they need, rows whole.
pub fn chunks(form: &Form, event: &str, records: &[&Record]) -> Vec<String> {
    let rows: Vec<String> = records.iter().filter(|r| r.event == event).map(|r| code_row(form, r)).collect();
    let set = hash(rows.join("\n").as_bytes())[..6].to_owned();
    let tag = form.tag();
    // The header's room, with three-digit counts.
    let header = format!("{MAGIC} 000/000 {set} {tag} {event}\n").len();
    let mut groups: Vec<Vec<&str>> = Vec::new();
    let mut size = header;
    for r in &rows {
        if groups.is_empty() || (size + r.len() + 1 > CODE_BYTES && !groups.last().is_some_and(|g| g.is_empty())) {
            groups.push(Vec::new());
            size = header;
        }
        if let Some(g) = groups.last_mut() {
            g.push(r);
        }
        size += r.len() + 1;
    }
    let n = groups.len();
    groups.iter().enumerate().map(|(i, g)| format!("{MAGIC} {}/{n} {set} {tag} {event}\n{}", i + 1, g.join("\n"))).collect()
}

/// One code read.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub index: usize,
    pub total: usize,
    pub set: String,
    pub records: Vec<Record>,
}

/// Whether a code's text is one of these.
pub fn is_code(text: &str) -> bool {
    text.trim_start().starts_with(MAGIC)
}

/// A code's text read, against this handheld's form.
pub fn parse(form: &Form, text: &str) -> Result<Chunk, String> {
    let mut lines = text.trim().lines();
    let head: Vec<&str> = lines.next().unwrap_or("").split_whitespace().collect();
    let [magic, place, set, tag, event] = head[..] else { return Err("not a scouting code".into()) };
    if magic != MAGIC {
        return Err("not a scouting code".into());
    }
    if tag != form.tag() {
        return Err("made with another form".into());
    }
    let (i, n) = place.split_once('/').ok_or("not a scouting code")?;
    let (index, total): (usize, usize) = (i.parse().map_err(|_| "unreadable")?, n.parse().map_err(|_| "unreadable")?);
    if index == 0 || index > total {
        return Err("unreadable".into());
    }
    let records: Vec<Record> = lines.filter(|l| !l.trim().is_empty()).map(|l| from_code_row(form, event, l).ok_or_else(|| "a row would not read".to_string())).collect::<Result<_, _>>()?;
    Ok(Chunk { index, total, set: set.to_owned(), records })
}

// ------------------------------------------------------------------- inbox ---

/// The codes of the set being read, as the camera reads them.
#[derive(Default, Debug)]
pub struct Inbox {
    pub set: String,
    pub total: usize,
    pub have: Vec<usize>,
    pub added: usize,
    pub updated: usize,
    pub problem: Option<String>,
}

impl Inbox {
    /// "3 of 7 codes", "all 7 codes".
    pub fn progress(&self) -> String {
        match (self.have.len(), self.total) {
            (h, n) if h >= n && n == 1 => "the code read".into(),
            (h, n) if h >= n => format!("all {n} codes read"),
            (h, n) => format!("{h} of {n} codes read"),
        }
    }

    /// "12 new · 2 newer", or "nothing new".
    pub fn outcome(&self) -> String {
        match (self.added, self.updated) {
            (0, 0) => "nothing new".into(),
            (a, 0) => format!("{a} new"),
            (0, u) => format!("{u} newer"),
            (a, u) => format!("{a} new · {u} newer"),
        }
    }

    /// Take a chunk in: merged into `store` unless this set's chunk was had.
    /// True when it was new.
    pub fn take(&mut self, chunk: Chunk, store: &mut super::store::Store) -> bool {
        if chunk.set != self.set {
            *self = Inbox { set: chunk.set.clone(), total: chunk.total, ..Inbox::default() };
        }
        self.problem = None;
        if self.have.contains(&chunk.index) {
            return false;
        }
        self.have.push(chunk.index);
        let records: Vec<Record> = chunk
            .records
            .into_iter()
            .map(|mut r| {
                r.history.push(Version { at: r.updated, by: r.by.clone(), change: "read from a code".into() });
                r
            })
            .collect();
        let (a, u) = store.merge_in(&records);
        self.added += a;
        self.updated += u;
        true
    }
}
