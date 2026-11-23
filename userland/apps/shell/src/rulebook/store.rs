//! The manual as kept on the card, in `/data/rulebook/`:
//!
//! - `manual.json`: where it came from, when, what changed last, and a row
//!   for each section and rule (its ID, title, where its text is). Read
//!   whole when the app opens: a few dozen kilobytes.
//! - `text.txt`: every body, one after another; a page reads its own.
//! - `index.bin`: the inverted index for search. Its dictionary is read
//!   when the first search is typed; each word's postings are read from the
//!   card as a search needs them.
//!
//! Search ranks with BM25 (words in a rule's headline count three times, a
//! rule's own ID five); the last word typed also matches as a prefix.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::parse::{self, normalize_id, Kind, Manual};

const MAGIC: &[u8; 4] = b"RBX1";
const K1: f32 = 1.2;
const B: f32 = 0.75;
/// A headline's words, and a rule's ID, weigh more than its body's.
const TITLE_WEIGHT: u32 = 3;
const ID_WEIGHT: u32 = 5;
/// Words a prefix stands for at most (the last word, still being typed).
const PREFIX_MOST: usize = 24;

/// Where the manual is kept: `/data/rulebook` on the handheld.
pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os("CATALYST_RULEBOOK") {
        return PathBuf::from(d);
    }
    if cfg!(target_os = "linux") && !cfg!(test) && Path::new("/data").is_dir() {
        PathBuf::from("/data/rulebook")
    } else {
        crate::config_dir().join("rulebook")
    }
}

// ------------------------------------------------------------------ words ---

const STOP: &[&str] = &[
    "a", "an", "the", "of", "to", "and", "or", "in", "on", "at", "for", "is", "are", "be", "by", "with", "as", "it", "its", "that", "this", "if", "from", "any", "can", "what", "how", "does", "do", "my", "we", "our", "you", "your", "when", "which", "there", "was", "were", "been", "has", "have", "into", "than", "then", "so", "such", "these", "those", "us", "i", "me", "about", "allowed", "rule", "rules",
];

/// A word as the index keeps it: lower case, a plural's "s" off; None for
/// the little words every rule has.
pub fn norm(word: &str) -> Option<String> {
    let w = word.to_lowercase();
    if STOP.contains(&w.as_str()) || (w.chars().count() == 1 && !w.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    Some(stem(w))
}

fn stem(mut w: String) -> String {
    let n = w.len();
    if n > 4 && w.ends_with("ies") {
        w.truncate(n - 3);
        w.push('y');
    } else if n > 4 && (w.ends_with("sses") || w.ends_with("ches") || w.ends_with("shes") || w.ends_with("xes")) {
        // "reaches", "boxes": the "es" off.
        w.truncate(n - 2);
    } else if n > 3 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") && !w.ends_with("is") {
        w.truncate(n - 1);
    }
    w
}

/// The words of a text, where each is: (from, to) in bytes.
pub fn spans(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        while chars.peek().is_some_and(|(_, c)| !c.is_alphanumeric()) {
            chars.next();
        }
        let (from, _) = *chars.peek()?;
        let mut to = from;
        while let Some(&(at, c)) = chars.peek() {
            if !c.is_alphanumeric() {
                break;
            }
            to = at + c.len_utf8();
            chars.next();
        }
        Some((from, to))
    })
}

/// The indexed words of a text.
pub fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    spans(text).filter_map(|(a, b)| norm(&text[a..b]))
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

// ------------------------------------------------------------------- kept ---

/// Where a manual came from and how it stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub title: String,
    pub source: String,
    /// The server's Last-Modified and ETag for it: whether it changed.
    pub modified: Option<String>,
    pub etag: Option<String>,
    /// When it was downloaded, and last looked at for a newer one.
    pub fetched: i64,
    pub checked: i64,
    /// The simulator's sample, not FIRST's manual.
    pub sample: bool,
    /// What the last update changed.
    pub changes: Option<Changes>,
}

/// What changed from one manual to the next: rules and sections by ID.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub at: i64,
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }

    pub fn count(&self) -> usize {
        self.changed.len() + self.added.len() + self.removed.len()
    }
}

/// One section or rule as kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub id: String,
    pub title: String,
    pub parent: Option<usize>,
    at: u64,
    len: u32,
    hash: u64,
    /// Its length in weighted words, for ranking.
    dl: u32,
}

impl Item {
    pub fn is_rule(&self) -> bool {
        self.kind == Kind::Rule
    }

    /// Its name in a list: "G101", or "7.1 Personal Safety".
    pub fn label(&self) -> String {
        match self.kind {
            Kind::Rule => self.id.clone(),
            _ if self.id.is_empty() => self.title.clone(),
            _ => format!("{} {}", self.id, self.title),
        }
    }
}

fn kind_name(k: &Kind) -> String {
    match k {
        Kind::Rule => "r".into(),
        Kind::Section(l) => l.to_string(),
    }
}

fn kind_from(s: &str) -> Kind {
    match s.parse::<u8>() {
        Ok(l) => Kind::Section(l),
        Err(_) => Kind::Rule,
    }
}

fn changes_json(c: &Changes) -> Value {
    json!({"at": c.at, "changed": c.changed, "added": c.added, "removed": c.removed})
}

fn changes_from(v: &Value) -> Option<Changes> {
    let list = |k: &str| v[k].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    v.is_object().then(|| Changes { at: v["at"].as_i64().unwrap_or(0), changed: list("changed"), added: list("added"), removed: list("removed") })
}

fn meta_json(meta: &Meta) -> Value {
    json!({
        "v": 1,
        "title": meta.title,
        "source": meta.source,
        "modified": meta.modified,
        "etag": meta.etag,
        "fetched": meta.fetched,
        "checked": meta.checked,
        "sample": meta.sample,
        "changes": meta.changes.as_ref().map(changes_json),
    })
}

fn meta_from(v: &Value) -> Meta {
    let s = |k: &str| v[k].as_str().map(str::to_owned);
    Meta {
        title: s("title").unwrap_or_default(),
        source: s("source").unwrap_or_default(),
        modified: s("modified"),
        etag: s("etag"),
        fetched: v["fetched"].as_i64().unwrap_or(0),
        checked: v["checked"].as_i64().unwrap_or(0),
        sample: v["sample"].as_bool().unwrap_or(false),
        changes: changes_from(&v["changes"]),
    }
}

/// What changed between the manual kept and the one replacing it.
pub fn diff(old: &[Item], new: &[Item], at: i64) -> Changes {
    let key = |i: &Item| if i.is_rule() { i.id.clone() } else { format!("{} {}", i.id, i.title).trim().to_owned() };
    let old_map: BTreeMap<String, u64> = old.iter().filter(|i| i.is_rule() || !i.id.is_empty()).map(|i| (key(i), i.hash)).collect();
    let new_map: BTreeMap<String, u64> = new.iter().filter(|i| i.is_rule() || !i.id.is_empty()).map(|i| (key(i), i.hash)).collect();
    // Rules first, in the manual's order; then sections.
    let order = |list: &[Item], wanted: &dyn Fn(&str) -> bool| -> Vec<String> {
        let mut rules: Vec<String> = list.iter().filter(|i| i.is_rule()).map(key).filter(|k| wanted(k)).collect();
        rules.extend(list.iter().filter(|i| !i.is_rule() && !i.id.is_empty()).map(key).filter(|k| wanted(k)));
        rules
    };
    Changes {
        at,
        changed: order(new, &|k| old_map.get(k).is_some_and(|h| Some(h) != new_map.get(k))),
        added: order(new, &|k| !old_map.contains_key(k)),
        removed: order(old, &|k| !new_map.contains_key(k)),
    }
}

/// Keep a manual in `dir`: its rows, its text and its index, each file
/// written whole before it replaces the one before. What changed from the
/// manual kept before goes into its `meta.changes` (unless it is the first).
pub fn write(dir: &Path, manual: &Manual, mut meta: Meta) -> Result<Meta, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let mut text = String::new();
    let mut items = Vec::with_capacity(manual.entries.len());
    let mut postings: BTreeMap<String, Vec<(u16, u8)>> = BTreeMap::new();
    let mut total = 0u64;
    for (doc, e) in manual.entries.iter().enumerate() {
        let at = text.len() as u64;
        text.push_str(&e.body);
        let mut tf: BTreeMap<String, u32> = BTreeMap::new();
        for w in words(&e.title) {
            *tf.entry(w).or_default() += TITLE_WEIGHT;
        }
        if e.kind == Kind::Rule {
            *tf.entry(e.id.to_lowercase()).or_default() += ID_WEIGHT;
        }
        for (_, para) in parse::paragraphs(&e.body) {
            for w in words(para) {
                *tf.entry(w).or_default() += 1;
            }
        }
        let dl: u32 = tf.values().sum();
        total += dl as u64;
        for (w, n) in tf {
            postings.entry(w).or_default().push((doc as u16, n.min(255) as u8));
        }
        let hash = fnv(&format!("{}\u{1}{}", e.title, e.body));
        items.push(Item { kind: e.kind.clone(), id: e.id.clone(), title: e.title.clone(), parent: e.parent, at, len: e.body.len() as u32, hash, dl });
    }
    // The index: the dictionary (each word, where its postings are, how many),
    // then the postings (document, weighted count).
    let mut dict = Vec::new();
    let mut post = Vec::new();
    for (w, list) in &postings {
        let bytes = w.as_bytes();
        if bytes.len() > 255 {
            continue;
        }
        dict.push(bytes.len() as u8);
        dict.extend_from_slice(bytes);
        dict.extend_from_slice(&((post.len() / 3) as u32).to_le_bytes());
        dict.extend_from_slice(&(list.len().min(u16::MAX as usize) as u16).to_le_bytes());
        for (doc, n) in list {
            post.extend_from_slice(&doc.to_le_bytes());
            post.push(*n);
        }
    }
    let avgdl = total as f32 / manual.entries.len().max(1) as f32;
    let mut index = Vec::with_capacity(20 + dict.len() + post.len());
    index.extend_from_slice(MAGIC);
    index.extend_from_slice(&(manual.entries.len() as u32).to_le_bytes());
    index.extend_from_slice(&(postings.len() as u32).to_le_bytes());
    index.extend_from_slice(&(dict.len() as u32).to_le_bytes());
    index.extend_from_slice(&avgdl.to_le_bytes());
    index.extend_from_slice(&dict);
    index.extend_from_slice(&post);

    let old = Book::open(dir).ok();
    if let Some(old) = &old {
        let changes = diff(&old.items, &items, meta.fetched);
        // An update that changed nothing keeps what the last one changed.
        meta.changes = if changes.is_empty() { old.meta.changes.clone() } else { Some(changes) };
    }
    let rows: Vec<Value> = items
        .iter()
        .map(|i| json!([kind_name(&i.kind), i.id, i.title, i.parent.map_or(-1, |p| p as i64), i.at, i.len, format!("{:016x}", i.hash), i.dl]))
        .collect();
    let mut kept = meta_json(&meta);
    kept["entries"] = Value::Array(rows);
    kept["manual_title"] = json!(manual.title);
    // The text and the index first: the rows that point into them last.
    crate::sys::write_atomic(&dir.join("text.txt"), text.as_bytes()).map_err(|e| format!("could not keep the text: {e}"))?;
    crate::sys::write_atomic(&dir.join("index.bin"), &index).map_err(|e| format!("could not keep the index: {e}"))?;
    crate::sys::write_atomic(&dir.join("manual.json"), kept.to_string().as_bytes()).map_err(|e| format!("could not keep the manual: {e}"))?;
    Ok(meta)
}

/// Note a look for a newer manual that found none.
pub fn checked(dir: &Path, at: i64) {
    let path = dir.join("manual.json");
    if let Some(mut v) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
        v["checked"] = json!(at);
        crate::sys::write_atomic(&path, v.to_string().as_bytes()).ok();
    }
}

/// Take the manual off this handheld (its bookmarks stay).
pub fn remove(dir: &Path) {
    for f in ["manual.json", "text.txt", "index.bin"] {
        std::fs::remove_file(dir.join(f)).ok();
    }
}

// ------------------------------------------------------------------- book ---

/// A search's result: an entry and its score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub entry: usize,
    pub score: f32,
}

/// What a search found, and the words it matched (to mark them).
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub hits: Vec<Hit>,
    pub terms: Vec<String>,
}

/// The index's dictionary, read once a search needs it.
struct Index {
    file: File,
    dict: Vec<u8>,
    /// Where each word starts in `dict`.
    offs: Vec<u32>,
    postings_at: u64,
    docs: usize,
    avgdl: f32,
}

impl Index {
    fn open(path: &Path) -> Result<Index, String> {
        let mut file = File::open(path).map_err(|_| "the index is missing".to_owned())?;
        let mut head = [0u8; 20];
        file.read_exact(&mut head).map_err(|_| "the index is damaged".to_owned())?;
        if &head[..4] != MAGIC {
            return Err("the index is from another version".into());
        }
        let u = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().expect("four bytes"));
        let (docs, terms, dict_len) = (u(4) as usize, u(8) as usize, u(12) as usize);
        let avgdl = f32::from_le_bytes(head[16..20].try_into().expect("four bytes"));
        let mut dict = vec![0u8; dict_len];
        file.read_exact(&mut dict).map_err(|_| "the index is damaged".to_owned())?;
        let mut offs = Vec::with_capacity(terms);
        let mut at = 0usize;
        while at < dict.len() && offs.len() < terms {
            offs.push(at as u32);
            at += 1 + dict[at] as usize + 6;
        }
        Ok(Index { file, dict, offs, postings_at: 20 + dict_len as u64, docs, avgdl: avgdl.max(1.0) })
    }

    fn term(&self, k: usize) -> &[u8] {
        let at = self.offs[k] as usize;
        &self.dict[at + 1..at + 1 + self.dict[at] as usize]
    }

    fn info(&self, k: usize) -> (u32, u16) {
        let at = self.offs[k] as usize;
        let end = at + 1 + self.dict[at] as usize;
        (u32::from_le_bytes(self.dict[end..end + 4].try_into().expect("four bytes")), u16::from_le_bytes(self.dict[end + 4..end + 6].try_into().expect("two bytes")))
    }

    /// The words starting with `p`, by their place in the dictionary.
    fn prefixed(&self, p: &[u8]) -> std::ops::Range<usize> {
        let (mut lo, mut hi) = (0, self.offs.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.term(mid) < p {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let mut hi = lo;
        while hi < self.offs.len() && self.term(hi).starts_with(p) {
            hi += 1;
        }
        lo..hi
    }

    fn exact(&self, w: &[u8]) -> Option<usize> {
        let r = self.prefixed(w);
        (r.start < r.end && self.term(r.start) == w).then_some(r.start)
    }

    fn postings(&mut self, k: usize) -> Vec<(u16, u8)> {
        let (at, n) = self.info(k);
        let mut buf = vec![0u8; n as usize * 3];
        if self.file.seek(SeekFrom::Start(self.postings_at + at as u64 * 3)).is_err() || self.file.read_exact(&mut buf).is_err() {
            return Vec::new();
        }
        buf.chunks_exact(3).map(|c| (u16::from_le_bytes([c[0], c[1]]), c[2])).collect()
    }
}

/// The manual kept on this handheld: its rows, read on opening; its text
/// and index, read as they are needed.
pub struct Book {
    pub meta: Meta,
    pub manual_title: String,
    pub items: Vec<Item>,
    dir: PathBuf,
    text: RefCell<Option<File>>,
    index: RefCell<Option<Index>>,
}

impl Book {
    pub fn open(dir: &Path) -> Result<Book, String> {
        let bytes = std::fs::read(dir.join("manual.json")).map_err(|_| "no manual on this handheld".to_owned())?;
        let v: Value = serde_json::from_slice(&bytes).map_err(|_| "the kept manual could not be read".to_owned())?;
        let items = v["entries"]
            .as_array()
            .ok_or("the kept manual has no rules")?
            .iter()
            .filter_map(|r| {
                let r = r.as_array()?;
                Some(Item {
                    kind: kind_from(r.first()?.as_str()?),
                    id: r.get(1)?.as_str()?.to_owned(),
                    title: r.get(2)?.as_str()?.to_owned(),
                    parent: r.get(3)?.as_i64().filter(|p| *p >= 0).map(|p| p as usize),
                    at: r.get(4)?.as_u64()?,
                    len: r.get(5)?.as_u64()? as u32,
                    hash: u64::from_str_radix(r.get(6)?.as_str()?, 16).ok()?,
                    dl: r.get(7)?.as_u64()? as u32,
                })
            })
            .collect();
        Ok(Book {
            meta: meta_from(&v),
            manual_title: v["manual_title"].as_str().unwrap_or("Game Manual").to_owned(),
            items,
            dir: dir.to_owned(),
            // Held open, so a manual replaced under it reads as it was.
            text: RefCell::new(File::open(dir.join("text.txt")).ok()),
            index: RefCell::new(None),
        })
    }

    /// An entry's body, read from the card.
    pub fn body(&self, i: usize) -> String {
        let Some(item) = self.items.get(i) else { return String::new() };
        let mut text = self.text.borrow_mut();
        let Some(file) = text.as_mut() else { return String::new() };
        let mut buf = vec![0u8; item.len as usize];
        if file.seek(SeekFrom::Start(item.at)).is_err() || file.read_exact(&mut buf).is_err() {
            return String::new();
        }
        String::from_utf8(buf).unwrap_or_default()
    }

    /// The rule with this ID, as typed ("g101", "G 101").
    pub fn find_id(&self, typed: &str) -> Option<usize> {
        let id = normalize_id(typed)?;
        self.items.iter().position(|i| i.is_rule() && i.id == id)
    }

    pub fn rules(&self) -> usize {
        self.items.iter().filter(|i| i.is_rule()).count()
    }

    /// The chapters: the sections at the top level.
    pub fn chapters(&self) -> Vec<usize> {
        (0..self.items.len()).filter(|&i| self.items[i].kind == Kind::Section(1)).collect()
    }

    /// Whether `i` is within section `s`.
    pub fn within(&self, mut i: usize, s: usize) -> bool {
        loop {
            match self.items[i].parent {
                Some(p) if p == s => return true,
                Some(p) => i = p,
                None => return false,
            }
        }
    }

    /// Everything in a chapter, in order.
    pub fn contents(&self, chapter: usize) -> Vec<usize> {
        (chapter + 1..self.items.len()).take_while(|&i| self.within(i, chapter)).collect()
    }

    /// Search the manual. A rule's ID finds that rule first; part of one
    /// ("G1") the rules it begins; words rank by BM25, a document that
    /// matches more of the words above one that matches fewer.
    pub fn search(&self, query: &str, limit: usize) -> Found {
        let mut hits: Vec<Hit> = Vec::new();
        if let Some(i) = self.find_id(query) {
            hits.push(Hit { entry: i, score: f32::MAX });
        } else if let Some(prefix) = partial_id(query) {
            hits.extend(self.items.iter().enumerate().filter(|(_, it)| it.is_rule() && it.id.starts_with(&prefix)).take(limit).enumerate().map(|(k, (i, _))| Hit { entry: i, score: 1e9 - k as f32 }));
        }
        let mut terms: Vec<String> = Vec::new();
        let typed: Vec<(String, Option<String>)> = query_terms(query);
        if typed.is_empty() {
            return Found { hits, terms };
        }
        let mut index = self.index.borrow_mut();
        if index.is_none() {
            *index = Index::open(&self.dir.join("index.bin")).ok();
        }
        let Some(ix) = index.as_mut() else { return Found { hits, terms } };
        let n = ix.docs.max(1) as f32;
        let mut score = vec![0f32; self.items.len()];
        let mut matched = vec![0u8; self.items.len()];
        for (word, prefix) in &typed {
            // This word's best score in each document, over what it stands for.
            let mut best: BTreeMap<u16, f32> = BTreeMap::new();
            let mut wanted: Vec<(usize, f32)> = ix.exact(word.as_bytes()).map(|k| (k, 1.0)).into_iter().collect();
            if let Some(p) = prefix {
                let exact = wanted.first().map(|(k, _)| *k);
                wanted.extend(ix.prefixed(p.as_bytes()).filter(|k| Some(*k) != exact).take(PREFIX_MOST).map(|k| (k, 0.7)));
            }
            for (k, weight) in wanted {
                terms.push(String::from_utf8_lossy(ix.term(k)).into_owned());
                let df = ix.info(k).1 as f32;
                let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
                for (doc, tf) in ix.postings(k) {
                    let Some(item) = self.items.get(doc as usize) else { continue };
                    let tf = tf as f32;
                    let s = weight * idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * item.dl as f32 / ix.avgdl));
                    let b = best.entry(doc).or_default();
                    *b = b.max(s);
                }
            }
            for (doc, s) in best {
                score[doc as usize] += s;
                matched[doc as usize] += 1;
            }
        }
        let many = typed.len() as f32;
        let mut ranked: Vec<Hit> = score
            .iter()
            .enumerate()
            .filter(|(i, s)| **s > 0.0 && !hits.iter().any(|h| h.entry == *i))
            .map(|(i, s)| Hit { entry: i, score: s * (1.0 + 0.5 * (matched[i] as f32 - 1.0) / (many - 1.0).max(1.0)) })
            .collect();
        ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.entry.cmp(&b.entry)));
        hits.extend(ranked);
        hits.truncate(limit);
        terms.sort();
        terms.dedup();
        Found { hits, terms }
    }
}

/// The start of a rule's ID, being typed: "G", "G1", "R20".
fn partial_id(query: &str) -> Option<String> {
    let q = query.trim().to_ascii_uppercase();
    let letters = q.chars().take_while(|c| c.is_ascii_uppercase()).count();
    let digits = q.chars().skip(letters).take_while(|c| c.is_ascii_digit()).count();
    ((1..=2).contains(&letters) && (1..=2).contains(&digits) && q.len() == letters + digits).then_some(q)
}

/// A query's words: each as the index keeps it, and for the last one
/// (still being typed, with no space after it) its start, to match as a
/// prefix too.
fn query_terms(query: &str) -> Vec<(String, Option<String>)> {
    let raw: Vec<(usize, usize)> = spans(query).collect();
    let typing = !query.ends_with(char::is_whitespace);
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    for (k, (a, b)) in raw.iter().enumerate() {
        let word = &query[*a..*b];
        let last = k + 1 == raw.len() && typing;
        let prefix = (last && word.chars().count() >= 2).then(|| word.to_lowercase());
        match norm(word) {
            Some(w) if !out.iter().any(|(o, _)| *o == w) => out.push((w, prefix)),
            None if prefix.is_some() && word.chars().count() >= 3 => out.push((word.to_lowercase(), prefix)),
            _ => {}
        }
    }
    out
}

/// Whether a word of a text is one of the words a search matched.
pub fn matches(word: &str, terms: &[String]) -> bool {
    norm(word).is_some_and(|w| terms.binary_search(&w).is_ok()) || terms.binary_search(&word.to_lowercase()).is_ok()
}

/// A line of a body around its first match, at most `room` characters:
/// (the words, where the matches are in it).
pub fn snippet(body: &str, terms: &[String], room: usize) -> (String, Vec<(usize, usize)>) {
    let paras: Vec<&str> = parse::paragraphs(body).map(|(_, t)| t).collect();
    let first = paras.iter().find_map(|p| spans(p).find(|(a, b)| matches(&p[*a..*b], terms)).map(|(a, _)| (*p, a)));
    let (para, at) = first.unwrap_or((paras.first().copied().unwrap_or(""), 0));
    // A few words before the match, from the start of a word.
    let mut from = at;
    let (mut back, mut whole) = (0, true);
    for (i, c) in para[..at].char_indices().rev() {
        if back >= room / 4 {
            whole = false;
            break;
        }
        back += 1;
        if c == ' ' {
            from = i + 1;
        }
    }
    if whole {
        from = 0;
    }
    let rest = &para[from..];
    let end = rest.char_indices().nth(room).map_or(rest.len(), |(i, _)| i);
    let mut text = String::new();
    if from > 0 {
        text.push('…');
    }
    let lead = text.len();
    text.push_str(&rest[..end]);
    if end < rest.len() {
        text.push('…');
    }
    let marks = spans(&rest[..end]).filter(|(a, b)| matches(&rest[*a..*b], terms)).map(|(a, b)| (a + lead, b + lead)).collect();
    (text, marks)
}
