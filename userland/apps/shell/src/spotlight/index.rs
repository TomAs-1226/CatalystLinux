//! Spotlight's index: everything it can find, gathered on a thread of its
//! own and held in memory, small, so a key never waits on the storage.
//!
//! Each provider (the apps, Settings, files, notes…) is asked for its items
//! once, then again only when what it reads has changed (`Provider::stamp`);
//! its part of the index is then swapped in whole and every other part stays
//! as it was. A search runs on the UI thread over the parts as they are just
//! then, and never waits for one being read.
//!
//! Names are matched the way a person reads them: the whole name, then its
//! start, then the start of a word in it, its initials, anywhere in it, the
//! other words it is known by, and (for the few short names of apps,
//! settings and actions) a close misspelling. What was picked lately is
//! remembered and found sooner.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::robot::Robot;
use crate::shell::{Cx, Screen};

/// How highly a kind of thing ranks against another found as well (lower
/// first). The gap between two is less than a step in how well the name
/// matched, so a better match always wins over a better kind.
pub mod weight {
    pub const APP: u8 = 0;
    pub const SETTING: u8 = 1;
    pub const ACTION: u8 = 2;
    pub const SCHEDULE: u8 = 2;
    pub const NOTE: u8 = 3;
    pub const CONTACT: u8 = 3;
    pub const REPO: u8 = 3;
    pub const VALUE: u8 = 4;
    pub const LOG: u8 = 4;
    /// What another app adds (reminders, music) when it has no better idea.
    pub const OTHER: u8 = 4;
    pub const PHOTO: u8 = 5;
    pub const FILE: u8 = 6;
}

/// Something Spotlight can find.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Its name among its provider's items, the same from one reading to the
    /// next: picks are remembered by it.
    pub id: String,
    /// What it is called: what is matched first, and the row's words.
    pub title: String,
    /// The quiet words at the right of its row: what it is, or a value.
    pub detail: String,
    /// One of the built-in symbols.
    pub icon: Cow<'static, str>,
    /// How it ranks against other kinds found as well (`weight`).
    pub weight: u8,
    /// Other words it is found by (a note's text, an app's other names),
    /// matched less highly than the title. Not kept once indexed.
    pub words: String,
    /// Found by a close misspelling too: for short names chosen from a few.
    pub fuzzy: bool,
    /// For an answer worked out from the query (`Provider::answer`): where
    /// it ranks, lower first (`Query::score` ranks a name as the index
    /// would). Not used for indexed items.
    pub rank: Option<u32>,
}

impl Item {
    pub fn new(id: impl Into<String>, title: impl Into<String>, icon: impl Into<Cow<'static, str>>, weight: u8) -> Self {
        Item { id: id.into(), title: title.into(), detail: String::new(), icon: icon.into(), weight, words: String::new(), fuzzy: false, rank: None }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn words(mut self, words: impl Into<String>) -> Self {
        self.words = words.into();
        self
    }

    pub fn fuzzy(mut self) -> Self {
        self.fuzzy = true;
        self
    }

    pub fn ranked(mut self, rank: u32) -> Self {
        self.rank = Some(rank);
        self
    }

}

/// An item as the index keeps it: its id, title and detail back to back in
/// one allocation, and nothing it is only found by (those words are kept
/// with the names it is matched by).
struct Stored {
    text: Box<str>,
    /// Where the id ends and the title, and where the title ends and the detail.
    cuts: [u32; 2],
    icon: Cow<'static, str>,
    weight: u8,
    fuzzy: bool,
}

impl Stored {
    fn new(item: Item) -> Self {
        let cuts = [item.id.len() as u32, (item.id.len() + item.title.len()) as u32];
        let text = [item.id, item.title, item.detail].concat().into_boxed_str();
        Stored { text, cuts, icon: item.icon, weight: item.weight, fuzzy: item.fuzzy }
    }

    fn id(&self) -> &str {
        &self.text[..self.cuts[0] as usize]
    }

    fn item(&self) -> Item {
        let [a, b] = self.cuts.map(|c| c as usize);
        Item {
            id: self.text[..a].to_owned(),
            title: self.text[a..b].to_owned(),
            detail: self.text[b..].to_owned(),
            icon: self.icon.clone(),
            weight: self.weight,
            words: String::new(),
            fuzzy: self.fuzzy,
            rank: None,
        }
    }

    /// The heap it holds, roughly: itself, its strings, and the allocator's
    /// share of each.
    fn bytes(&self) -> usize {
        let icon = match &self.icon {
            Cow::Borrowed(_) => 0,
            Cow::Owned(s) => s.len() + 16,
        };
        std::mem::size_of::<Stored>() + self.text.len() + 16 + icon
    }
}

/// What Tab shows of a result.
#[derive(Clone, Debug)]
pub enum Preview {
    /// The first lines of a file or a note, and a few facts about it.
    Text { lines: Vec<String>, facts: Vec<(String, String)> },
    /// A photo, small, and its facts.
    Picture { picture: crate::photos::Picture, facts: Vec<(String, String)> },
    /// Facts alone: what it is, where, how big.
    Facts(Vec<(String, String)>),
}

/// A source of results. Built-in ones find the apps, Settings, files, notes
/// and the rest; any app can add its own with [`register`].
///
/// Two kinds of finding, either or both:
/// - **Indexed** (`items`): what the provider has, read on the index's own
///   thread (the disk is fine there, never on the UI thread) and searched
///   from memory. Read again only when `stamp` says it changed.
/// - **Answers** (`answer`): worked out from the query itself, on the UI
///   thread as each key lands (a sum, a match number, a robot value). These
///   must be quick and must never touch the disk.
pub trait Provider: Send + Sync {
    /// A short name, unique among providers ("notes"): picks are remembered
    /// under it.
    fn name(&self) -> &'static str;

    /// Everything it can find. Called on the index's thread, at start and
    /// whenever `stamp` changes.
    fn items(&self) -> Vec<Item> {
        Vec::new()
    }

    /// A cheap fingerprint of what `items` reads (a folder's change time),
    /// asked on the index's thread each time Spotlight is summoned and every
    /// few seconds while it shows: `items` is read again when it differs
    /// from the one asked just after the last reading.
    /// `None`: no cheap way to tell, so `items` is read again when Spotlight
    /// is summoned and the last reading is over half a minute old.
    fn stamp(&self) -> Option<u64> {
        None
    }

    /// Results worked out from the query, each with `Item::rank` set. On the
    /// UI thread: quick, and never the disk.
    fn answer(&self, _query: &Query, _robot: &Robot, _out: &mut Vec<Item>) {}

    /// The screen a result opens, on the UI thread. `Err` is said in the island.
    fn open(&self, item: &Item, cx: &mut Cx) -> Result<Box<dyn Screen>, String>;

    /// What Tab shows of it, read on a thread of its own (the disk is fine).
    fn preview(&self, _item: &Item) -> Option<Preview> {
        None
    }

    /// What Tab shows of it from what is in memory (a robot value), on the
    /// UI thread. Tried before `preview`.
    fn live_preview(&self, _item: &Item, _robot: &Robot) -> Option<Preview> {
        None
    }

    /// The memory it keeps beside its items (a cache of folders), roughly,
    /// for the index's measure of itself.
    fn bytes(&self) -> usize {
        0
    }
}

// --------------------------------------------------------------- matching ---

/// Lower case, for matching.
pub fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// A name as words: lower case, split where it has anything but letters and
/// digits and where a capital starts a word ("BatteryVolts" is "battery
/// volts"), single spaces between.
pub fn split_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if c.is_alphanumeric() {
            let boundary = prev.is_some_and(|p| (p.is_lowercase() && c.is_uppercase()) || (p.is_alphabetic() && c.is_ascii_digit()) || (p.is_ascii_digit() && c.is_alphabetic()));
            if boundary && !out.ends_with(' ') && !out.is_empty() {
                out.push(' ');
            }
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        prev = Some(c);
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// What was typed, ready to match: lower case, single spaces, its words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// As typed, trimmed (for answers that read it: a sum, a unit).
    pub raw: String,
    /// Lower case, single spaces.
    pub text: String,
    pub parts: Vec<String>,
    /// Without its spaces, for initials and misspellings.
    pub squashed: String,
}

/// How a name matched, best first. Each step is worth more than any kind or
/// length, so a better match always ranks higher.
const TIER: u32 = 300;
const EXACT: u32 = 0;
const PREFIX: u32 = 1;
const WORD: u32 = 2;
const INITIALS: u32 = 3;
const INSIDE: u32 = 4;
const OTHER_WORDS: u32 = 5;
const MISSPELT: u32 = 6;

/// The most a pick can lift a result: about one step of match, so what is
/// picked often comes first among close matches but never over a far better one.
const BOOST_MOST: f32 = 450.0;

/// Ranks at or past this are the rows that always come last (the web, the assistant).
pub const TAIL: u32 = u32::MAX - 16;

impl Query {
    pub fn new(typed: &str) -> Self {
        let raw = typed.trim().to_owned();
        let parts: Vec<String> = fold(&raw).split_whitespace().map(str::to_owned).collect();
        Query { text: parts.join(" "), squashed: parts.concat(), parts, raw }
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// How well a name matches: its step (`EXACT` … `MISSPELT`) and where in
    /// it the match starts. `title` is the name in lower case, `split` its
    /// words (`split_words`), `words` the other words it is known by.
    fn tier(&self, title: &str, split: &str, words: &str, fuzzy: bool) -> Option<(u32, u32)> {
        let q = self.text.as_str();
        if q.is_empty() {
            return None;
        }
        if title == q || split == q {
            return Some((EXACT, 0));
        }
        if title.starts_with(q) || split.starts_with(q) {
            return Some((PREFIX, 0));
        }
        let mut name_words = split.split(' ');
        if self.parts.len() == 1 {
            if let Some(at) = word_starting(split, q) {
                return Some((WORD, at as u32));
            }
        } else if self.parts.iter().all(|p| name_words.clone().any(|w| w.starts_with(p.as_str()))) {
            return Some((WORD, 1));
        }
        if self.squashed.chars().count() >= 2 && self.parts.len() == 1 {
            let mut initials = name_words.by_ref().filter_map(|w| w.chars().next());
            if self.squashed.chars().all(|c| initials.next() == Some(c)) {
                return Some((INITIALS, 0));
            }
        }
        // One letter anywhere in a name says nothing: from two on.
        if self.squashed.chars().count() < 2 {
            return None;
        }
        if let Some(at) = title.find(q) {
            return Some((INSIDE, at as u32));
        }
        if self.parts.len() > 1 && self.parts.iter().all(|p| title.contains(p.as_str())) {
            return Some((INSIDE, 1));
        }
        if !words.is_empty() && self.parts.iter().all(|p| title.contains(p.as_str()) || word_starting(words, p).is_some() || (p.len() >= 3 && words.contains(p.as_str()))) {
            return Some((OTHER_WORDS, 0));
        }
        if fuzzy && self.squashed.chars().count() >= 3 && misspelt(title, &self.squashed) {
            return Some((MISSPELT, 0));
        }
        None
    }

    /// Where a name ranks for this query, as the index ranks its items
    /// (lower first), or `None` if it does not match. For answers.
    pub fn score(&self, title: &str, words: &str, weight: u8, fuzzy: bool) -> Option<u32> {
        self.rank(&fold(title), &split_words(title), &fold(words), weight, fuzzy)
    }

    /// As `score`, for names already made ready (`fold`, `split_words`):
    /// for an answer that keeps many names and ranks them at every key.
    pub fn rank(&self, lower: &str, split: &str, words: &str, weight: u8, fuzzy: bool) -> Option<u32> {
        let (tier, at) = self.tier(lower, split, words, fuzzy)?;
        Some(score(tier, at, weight, lower.chars().count(), 0))
    }
}

/// Which letters and digits a text holds, a bit each (anything else shares
/// one): a name that lacks a letter typed cannot match it but by a
/// misspelling, and is passed over without a look at its words.
fn letters(text: &str) -> u64 {
    text.chars().fold(0, |mask, c| {
        mask | match c {
            'a'..='z' => 1 << (c as u32 - 'a' as u32),
            '0'..='9' => 1 << (26 + c as u32 - '0' as u32),
            ' ' => 0,
            _ => 1 << 63,
        }
    })
}

/// Where the first word of `words` (single spaces) starting with `q` starts.
fn word_starting(words: &str, q: &str) -> Option<usize> {
    let mut at = 0;
    for w in words.split(' ') {
        if w.starts_with(q) {
            return Some(at);
        }
        at += w.len() + 1;
    }
    None
}

/// `q`'s letters in `name` in order, close together: a letter left out, or
/// one too many, still finds an app ("termnal", "setings"). Not letters
/// scattered across a long name.
fn misspelt(name: &str, q: &str) -> bool {
    let name: Vec<char> = name.chars().filter(|c| !c.is_whitespace()).collect();
    let q: Vec<char> = q.chars().collect();
    // In order: every letter of the query found, starting with the name's first.
    let mut at = 0;
    let mut first = None;
    for c in &q {
        match name[at..].iter().position(|n| n == c) {
            Some(i) => {
                first.get_or_insert(at + i);
                at += i + 1;
            }
            None => return near_miss(&name, &q),
        }
    }
    let span = at - first.unwrap_or(0);
    (first == Some(0) && span <= q.len() + q.len() / 2 + 1) || near_miss(&name, &q)
}

/// One letter wrong, or two swapped, in the name's first `q.len()` letters.
fn near_miss(name: &[char], q: &[char]) -> bool {
    if name.len() < q.len() || q.len() < 3 {
        return false;
    }
    let head = &name[..q.len()];
    let wrong: Vec<usize> = (0..q.len()).filter(|&i| head[i] != q[i]).collect();
    match wrong.as_slice() {
        [_] => true,
        [a, b] => b - a == 1 && head[*a] == q[*b] && head[*b] == q[*a],
        _ => false,
    }
}

/// The rank of a match: its step, its kind, where it starts, how long the
/// name is, less what picking it lately is worth.
fn score(tier: u32, at: u32, weight: u8, len: usize, boost: u32) -> u32 {
    let base = tier * TIER + u32::from(weight.min(6)) * 40 + at.min(10) * 2 + (len as u32).min(40);
    base.saturating_sub(boost)
}

// ------------------------------------------------------------------ picks ---

/// One thing picked: how often, and when last (Unix seconds).
#[derive(Clone, Debug, PartialEq)]
struct Pick {
    key: String,
    count: u32,
    last: i64,
}

/// What was picked lately, kept beside the settings.
#[derive(Default)]
pub struct Picks {
    list: Vec<Pick>,
    loaded: bool,
}

/// How many picks are kept.
const PICKS_KEPT: usize = 200;

/// A pick's worth halves every two weeks.
const HALF_LIFE_DAYS: f32 = 14.0;

fn picks_file() -> std::path::PathBuf {
    crate::config_dir().join("spotlight-picks")
}

impl Picks {
    fn parse(text: &str) -> Vec<Pick> {
        text.lines()
            .filter_map(|line| {
                let mut f = line.splitn(3, '\t');
                let count = f.next()?.parse().ok()?;
                let last = f.next()?.parse().ok()?;
                let key = f.next()?.to_owned();
                (!key.is_empty()).then_some(Pick { key, count, last })
            })
            .collect()
    }

    fn text(&self) -> String {
        self.list.iter().map(|p| format!("{}\t{}\t{}\n", p.count, p.last, p.key)).collect()
    }

    /// Read from the disk, once (on the index's thread), after any picked
    /// meanwhile.
    fn load(&mut self) {
        if !self.loaded {
            for kept in Self::parse(&std::fs::read_to_string(picks_file()).unwrap_or_default()) {
                if !self.list.iter().any(|p| p.key == kept.key) {
                    self.list.push(kept);
                }
            }
            self.list.truncate(PICKS_KEPT);
            self.loaded = true;
        }
    }

    fn remember(&mut self, key: &str, now: i64) {
        match self.list.iter().position(|p| p.key == key) {
            Some(at) => {
                let mut pick = self.list.remove(at);
                pick.count = pick.count.saturating_add(1);
                pick.last = now;
                self.list.insert(0, pick);
            }
            None => self.list.insert(0, Pick { key: key.to_owned(), count: 1, last: now }),
        }
        self.list.truncate(PICKS_KEPT);
    }

    /// What a pick lifts its result by: more for one picked often and lately.
    fn boost(pick: &Pick, now: i64) -> u32 {
        let days = ((now - pick.last).max(0) as f32) / 86_400.0;
        let worth = 150.0 * (1.0 + pick.count as f32).log2() * 0.5f32.powf(days / HALF_LIFE_DAYS);
        worth.min(BOOST_MOST) as u32
    }
}

// ------------------------------------------------------------------ index ---

/// One provider's part of the index: its items, and their names in lower
/// case and as words back to back in one string (one allocation, not three
/// an item).
struct Segment {
    stamp: Option<u64>,
    read: Instant,
    items: Vec<Stored>,
    text: String,
    /// Per item: where its lower-case title, its words-split title and its
    /// other words start in `text`, and where they end.
    spans: Vec<[u32; 4]>,
    /// Per item: the letters its names hold (`letters`).
    masks: Vec<u64>,
    /// What the provider keeps beside its items (a folder cache).
    kept: usize,
}

impl Segment {
    fn new(stamp: Option<u64>, items: Vec<Item>, kept: usize) -> Self {
        let mut text = String::new();
        let mut spans = Vec::with_capacity(items.len());
        let mut masks = Vec::with_capacity(items.len());
        let mut stored = Vec::with_capacity(items.len());
        for item in items {
            let a = text.len() as u32;
            text.push_str(&fold(&item.title));
            let b = text.len() as u32;
            text.push_str(&split_words(&item.title));
            let c = text.len() as u32;
            // The other words, in lower case and a kilobyte at most.
            let words = fold(&item.words);
            let mut end = words.len().min(1024);
            while !words.is_char_boundary(end) {
                end -= 1;
            }
            text.push_str(&words[..end]);
            spans.push([a, b, c, text.len() as u32]);
            masks.push(letters(&text[a as usize..]));
            stored.push(Stored::new(item));
        }
        text.shrink_to_fit();
        Segment { stamp, read: Instant::now(), items: stored, text, spans, masks, kept }
    }

    fn names(&self, i: usize) -> (&str, &str, &str) {
        let [a, b, c, d] = self.spans[i].map(|x| x as usize);
        (&self.text[a..b], &self.text[b..c], &self.text[c..d])
    }

    fn bytes(&self) -> usize {
        self.items.iter().map(Stored::bytes).sum::<usize>() + self.text.capacity() + self.spans.capacity() * 16 + self.masks.capacity() * 8 + std::mem::size_of::<Self>() + self.kept
    }
}

/// The parts as they are at one moment: one for each provider, by its place.
#[derive(Default)]
struct Snapshot {
    segments: Vec<Option<Arc<Segment>>>,
}

/// A result: which provider found it, the item, and its rank.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub provider: usize,
    pub item: Item,
    pub score: u32,
}

/// How big the index is, and how long it took to read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub items: usize,
    /// Its heap, roughly (strings with the allocator's share, the tables).
    pub bytes: usize,
    /// How long the last full reading took, every provider (ms).
    pub read_ms: f32,
}

#[derive(Default)]
struct Wake {
    poked: bool,
    /// Spotlight is showing: look for changes every few seconds.
    active: bool,
}

pub struct Index {
    providers: Mutex<Vec<Arc<dyn Provider>>>,
    snapshot: Mutex<Arc<Snapshot>>,
    generation: AtomicU64,
    wake: Mutex<Wake>,
    bell: Condvar,
    picks: Mutex<Picks>,
    stats: Mutex<Stats>,
}

/// A part read longer ago than this, with no stamp to tell by, is read again
/// when Spotlight is summoned.
const REFRESH: Duration = Duration::from_secs(30);
/// While Spotlight shows, the stamps are looked at this often.
const WATCH: Duration = Duration::from_secs(3);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Index {
    pub fn new(providers: Vec<Arc<dyn Provider>>) -> Self {
        Index {
            providers: Mutex::new(providers),
            snapshot: Mutex::new(Arc::new(Snapshot::default())),
            generation: AtomicU64::new(0),
            wake: Mutex::new(Wake::default()),
            bell: Condvar::new(),
            picks: Mutex::new(Picks::default()),
            stats: Mutex::new(Stats::default()),
        }
    }

    /// Add a provider; its items are read on the index's thread straight away.
    pub fn register(&self, provider: Arc<dyn Provider>) {
        let mut providers = lock(&self.providers);
        if providers.iter().any(|p| p.name() == provider.name()) {
            return;
        }
        providers.push(provider);
        drop(providers);
        self.poke();
    }

    fn providers(&self) -> Vec<Arc<dyn Provider>> {
        lock(&self.providers).clone()
    }

    fn snapshot(&self) -> Arc<Snapshot> {
        lock(&self.snapshot).clone()
    }

    /// Changes each time a part of the index is swapped in.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Look for changes now (Spotlight was summoned).
    pub fn poke(&self) {
        lock(&self.wake).poked = true;
        self.bell.notify_all();
    }

    /// Spotlight is showing, or not: while it is, changes are looked for
    /// every few seconds; otherwise the thread sleeps until poked.
    pub fn set_active(&self, active: bool) {
        lock(&self.wake).active = active;
        if active {
            self.bell.notify_all();
        }
    }

    /// Its size and how long it took to read (the measurements, and a
    /// provider's author checking theirs).
    #[allow(dead_code)]
    pub fn stats(&self) -> Stats {
        *lock(&self.stats)
    }

    /// The index's thread: read what changed whenever poked (and every few
    /// seconds while Spotlight shows), at a lower priority than the frames.
    pub fn run(self: Arc<Self>) {
        lower_priority();
        lock(&self.picks).load();
        loop {
            let poked = {
                let mut wake = lock(&self.wake);
                loop {
                    if wake.poked {
                        break;
                    }
                    let timeout = if wake.active { WATCH } else { Duration::from_secs(3600) };
                    let (w, waited) = self.bell.wait_timeout(wake, timeout).unwrap_or_else(|e| e.into_inner());
                    wake = w;
                    if waited.timed_out() && wake.active {
                        break;
                    }
                }
                std::mem::take(&mut wake.poked)
            };
            self.refresh(poked);
        }
    }

    /// Read again every part whose provider says it changed (or that was
    /// never read). `poked`: Spotlight was just summoned, so a part with no
    /// stamp is read again too if it is old. True if anything was swapped in.
    pub fn refresh(&self, poked: bool) -> bool {
        let began = Instant::now();
        let mut changed = false;
        let providers = self.providers();
        let mut all_read = true;
        for (at, provider) in providers.iter().enumerate() {
            let old = self.snapshot().segments.get(at).cloned().flatten();
            let stamp = provider.stamp();
            let due = match (&old, stamp) {
                (None, _) => true,
                (Some(seg), Some(s)) => seg.stamp != Some(s),
                (Some(seg), None) => poked && seg.read.elapsed() > REFRESH,
            };
            if !due {
                all_read = false;
                continue;
            }
            // Kept with the stamp as it is once read: what was read is what
            // the next stamp is measured against.
            let items = provider.items();
            let segment = Arc::new(Segment::new(provider.stamp(), items, provider.bytes()));
            // Swapped in on its own, so what is quick to read shows first.
            {
                let mut snap = lock(&self.snapshot);
                let mut segments = snap.segments.clone();
                if segments.len() <= at {
                    segments.resize(at + 1, None);
                }
                segments[at] = Some(segment);
                *snap = Arc::new(Snapshot { segments });
            }
            self.generation.fetch_add(1, Ordering::Relaxed);
            changed = true;
        }
        let snap = self.snapshot();
        let mut stats = lock(&self.stats);
        stats.items = snap.segments.iter().flatten().map(|s| s.items.len()).sum();
        stats.bytes = snap.segments.iter().flatten().map(|s| s.bytes()).sum::<usize>() + snap.segments.capacity() * 8;
        if changed && all_read {
            stats.read_ms = began.elapsed().as_secs_f32() * 1000.0;
        }
        changed
    }

    /// What each provider's picks lift their items by, for one search.
    fn boosts(&self, providers: &[Arc<dyn Provider>]) -> Vec<HashMap<String, u32>> {
        let now = now_secs();
        let picks = lock(&self.picks);
        let mut boosts: Vec<HashMap<String, u32>> = vec![HashMap::new(); providers.len()];
        for pick in &picks.list {
            let Some((name, id)) = pick.key.split_once(':') else { continue };
            if let Some(at) = providers.iter().position(|p| p.name() == name) {
                boosts[at].insert(id.to_owned(), Picks::boost(pick, now));
            }
        }
        boosts
    }

    /// Everything that matches `query`, best first: at most `limit` of the
    /// index's and the answers', then the rows that always come last.
    pub fn search(&self, query: &Query, robot: &Robot, limit: usize) -> Vec<Hit> {
        if query.is_empty() {
            return Vec::new();
        }
        let snap = self.snapshot();
        let providers = self.providers();
        let boosts = self.boosts(&providers);
        // (rank, provider, item) for every match: only the best are copied out.
        let mut found: Vec<(u32, u16, u32)> = Vec::new();
        let typed = letters(&query.squashed);
        for (p, seg) in snap.segments.iter().enumerate() {
            let Some(seg) = seg else { continue };
            for (i, item) in seg.items.iter().enumerate() {
                // Lacking a letter typed, only a misspelling could match.
                if seg.masks[i] & typed != typed && !item.fuzzy {
                    continue;
                }
                let (title, split, words) = seg.names(i);
                if let Some((tier, at)) = query.tier(title, split, words, item.fuzzy) {
                    let boost = if boosts[p].is_empty() { 0 } else { boosts[p].get(item.id()).copied().unwrap_or(0) };
                    found.push((score(tier, at, item.weight, title.len(), boost), p as u16, i as u32));
                }
            }
        }
        if found.len() > limit {
            found.select_nth_unstable(limit);
            found.truncate(limit);
        }
        let mut hits: Vec<Hit> = found
            .iter()
            .filter_map(|&(score, p, i)| {
                let item = snap.segments.get(p as usize)?.as_ref()?.items.get(i as usize)?.item();
                Some(Hit { provider: p as usize, item, score })
            })
            .collect();
        // The answers, worked out from the query.
        let mut tail = Vec::new();
        for (p, provider) in providers.iter().enumerate() {
            let mut out = Vec::new();
            provider.answer(query, robot, &mut out);
            for item in out {
                let rank = item.rank.unwrap_or(TAIL - 1);
                let boost = if rank < TAIL { boosts[p].get(item.id.as_str()).copied().unwrap_or(0) } else { 0 };
                let hit = Hit { provider: p, score: rank.saturating_sub(boost), item };
                if rank >= TAIL {
                    tail.push(hit);
                } else {
                    hits.push(hit);
                }
            }
        }
        // Ties go by name, so the order is the same however they were found.
        hits.sort_by_cached_key(|h| (h.score, fold(&h.item.title)));
        hits.truncate(limit);
        tail.sort_by_key(|h| h.score);
        hits.extend(tail);
        hits
    }

    /// The things picked lately that the index still holds, latest first.
    pub fn recent(&self, limit: usize) -> Vec<Hit> {
        let snap = self.snapshot();
        let providers = self.providers();
        let picks: Vec<Pick> = lock(&self.picks).list.clone();
        let mut hits = Vec::new();
        for pick in picks {
            if hits.len() >= limit {
                break;
            }
            let Some((name, id)) = pick.key.split_once(':') else { continue };
            let Some(p) = providers.iter().position(|pr| pr.name() == name) else { continue };
            let Some(Some(seg)) = snap.segments.get(p) else { continue };
            if let Some(item) = seg.items.iter().find(|it| it.id() == id) {
                hits.push(Hit { provider: p, item: item.item(), score: 0 });
            }
        }
        hits
    }

    /// Open a result, and remember that it was picked.
    pub fn open(&self, hit: &Hit, cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let provider = self.providers().get(hit.provider).cloned().ok_or("that is gone")?;
        let screen = provider.open(&hit.item, cx)?;
        // The web and the assistant rows are not picks worth remembering.
        if hit.item.rank.is_none_or(|r| r < TAIL) {
            self.picked(&format!("{}:{}", provider.name(), hit.item.id));
        }
        Ok(screen)
    }

    /// Remember a pick, and keep it on the disk (on a thread: never the UI's).
    /// Picked before the index's thread has read the picks kept, it is added
    /// to them there, never written over them.
    pub fn picked(&self, key: &str) {
        let now = now_secs();
        let (text, loaded) = {
            let mut picks = lock(&self.picks);
            picks.remember(key, now);
            (picks.text(), picks.loaded)
        };
        let key = key.to_owned();
        std::thread::Builder::new()
            .name("spotlight-picks".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let text = if loaded {
                    text
                } else {
                    let mut kept = Picks::default();
                    kept.load();
                    kept.remember(&key, now);
                    kept.text()
                };
                std::fs::create_dir_all(crate::config_dir()).ok();
                crate::sys::write_atomic(&picks_file(), text.as_bytes()).ok();
            })
            .ok();
    }

    /// The provider behind a result, for its preview.
    pub fn provider(&self, at: usize) -> Option<Arc<dyn Provider>> {
        self.providers().get(at).cloned()
    }
}

/// The index's thread gives way to the frames: on the handheld, at a lower
/// priority than the UI thread.
fn lower_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain system calls on this thread's own id.
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static str, Vec<Item>);

    impl Provider for Fixed {
        fn name(&self) -> &'static str {
            self.0
        }
        fn items(&self) -> Vec<Item> {
            self.1.clone()
        }
        fn stamp(&self) -> Option<u64> {
            Some(1)
        }
        fn open(&self, _item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
            Err("not in a test".into())
        }
    }

    fn index_of(items: Vec<Item>) -> Index {
        let index = Index::new(vec![Arc::new(Fixed("test", items))]);
        index.refresh(true);
        index
    }

    fn titles(index: &Index, typed: &str) -> Vec<String> {
        index.search(&Query::new(typed), &Robot::none(), 20).into_iter().map(|h| h.item.title).collect()
    }

    #[test]
    fn names_are_split_into_words_as_they_are_read() {
        assert_eq!(split_words("Wi-Fi networks"), "wi fi networks");
        assert_eq!(split_words("Systemcore/BatteryVolts"), "systemcore battery volts");
        assert_eq!(split_words("IMG_20261116_101500.jpg"), "img 20261116 101500 jpg");
        assert_eq!(split_words("  Net   tools "), "net tools");
        assert_eq!(split_words("q14"), "q 14");
    }

    #[test]
    fn a_match_ranks_by_where_the_query_is_found() {
        let q = |s: &str| Query::new(s);
        let tier = |name: &str, typed: &str| q(typed).tier(&fold(name), &split_words(name), "", false).map(|t| t.0);
        assert_eq!(tier("Logs", "logs"), Some(EXACT));
        assert_eq!(tier("Wi-Fi networks", "wi"), Some(PREFIX));
        assert_eq!(tier("Wi-Fi networks", "net"), Some(WORD));
        assert_eq!(tier("Wi-Fi networks", "wi net"), Some(WORD), "each word typed starts a word");
        assert_eq!(tier("Net tools", "nt"), Some(INITIALS));
        assert_eq!(tier("Wi-Fi networks", "work"), Some(INSIDE));
        assert_eq!(tier("Wi-Fi networks", "zz"), None);
        assert_eq!(tier("Systemcore/BatteryVolts", "volts"), Some(WORD), "a capital starts a word");
        assert_eq!(q("  ").tier("anything", "anything", "", false), None);
        // Other words: found, but below the name itself.
        assert_eq!(q("df").tier("disk space", "disk space", "df -h", false).map(|t| t.0), Some(OTHER_WORDS));
        // A misspelling finds only what may be misspelt, and only once three letters are typed.
        let fuzzy = |name: &str, typed: &str| q(typed).tier(&fold(name), &split_words(name), "", true).map(|t| t.0);
        assert_eq!(fuzzy("Terminal", "termnal"), Some(MISSPELT));
        assert_eq!(fuzzy("Settings", "setings"), Some(MISSPELT));
        assert_eq!(fuzzy("Settings", "sertings"), Some(MISSPELT), "one letter wrong");
        assert_eq!(fuzzy("Settings", "sb"), None);
        assert_eq!(q("termnal").tier("terminal", "terminal", "", false), None, "not for files");
        assert_eq!(fuzzy("Processor temperature", "pte"), None, "not letters scattered across a long name");
    }

    #[test]
    fn better_matches_come_first_then_kinds_then_shorter_names() {
        let index = index_of(vec![
            Item::new("a", "Wi-Fi networks", "wifi", weight::SETTING),
            Item::new("b", "Net tools", "network_check", weight::APP),
            Item::new("c", "network.txt", "description", weight::FILE),
            Item::new("d", "Internet reachable", "play_circle", weight::ACTION),
            Item::new("e", "Network", "lan", weight::SETTING),
        ]);
        assert_eq!(titles(&index, "net"), ["Net tools", "Network", "network.txt", "Wi-Fi networks", "Internet reachable"]);
        // Typed in full: the exact name first, whatever its kind.
        assert_eq!(titles(&index, "network")[0], "Network");
    }

    #[test]
    fn what_is_picked_often_comes_first_among_close_matches() {
        let index = index_of(vec![
            Item::new("tables", "Tables", "account_tree", weight::APP),
            Item::new("terminal", "Terminal", "terminal", weight::APP),
            Item::new("data", "Data tables", "account_tree", weight::APP),
            Item::new("t.txt", "t.txt", "description", weight::FILE),
        ]);
        assert_eq!(titles(&index, "t"), ["Tables", "Terminal", "t.txt", "Data tables"]);
        // Picked three times: Terminal comes first among the names starting so.
        for _ in 0..3 {
            lock(&index.picks).remember("test:terminal", now_secs());
        }
        assert_eq!(titles(&index, "t")[0], "Terminal");
        // Picked once, a name only a word of which starts so does not jump a
        // name that starts so.
        lock(&index.picks).remember("test:data", now_secs());
        assert_eq!(titles(&index, "tab"), ["Tables", "Data tables"]);
        // With nothing typed, the picks are offered, latest first.
        assert_eq!(index.recent(5).iter().map(|h| h.item.title.as_str()).collect::<Vec<_>>(), ["Data tables", "Terminal"]);
    }

    #[test]
    fn a_pick_is_worth_less_as_it_ages() {
        let now = 1_800_000_000;
        let fresh = Pick { key: "a:b".into(), count: 3, last: now };
        let old = Pick { last: now - 28 * 86_400, ..fresh.clone() };
        assert_eq!(Picks::boost(&fresh, now), 300);
        assert_eq!(Picks::boost(&old, now), 75, "a quarter after four weeks");
        let many = Pick { count: 1000, ..fresh.clone() };
        assert_eq!(Picks::boost(&many, now), BOOST_MOST as u32, "never more than a step of match");
        // Kept and read back as they were.
        let mut picks = Picks::default();
        picks.remember("notes:a.txt", now);
        picks.remember("apps:Git", now + 5);
        picks.remember("notes:a.txt", now + 9);
        assert_eq!(Picks::parse(&picks.text()), picks.list);
        assert_eq!(picks.list[0], Pick { key: "notes:a.txt".into(), count: 2, last: now + 9 });
    }

    #[test]
    fn a_part_is_read_again_only_when_its_stamp_changes() {
        use std::sync::atomic::AtomicU32;
        struct Counting(AtomicU32, AtomicU64);
        impl Provider for Counting {
            fn name(&self) -> &'static str {
                "counting"
            }
            fn items(&self) -> Vec<Item> {
                let n = self.0.fetch_add(1, Ordering::Relaxed);
                vec![Item::new(format!("{n}"), format!("Reading {n}"), "notes", weight::NOTE)]
            }
            fn stamp(&self) -> Option<u64> {
                Some(self.1.load(Ordering::Relaxed))
            }
            fn open(&self, _item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
                Err(String::new())
            }
        }
        let counting = Arc::new(Counting(AtomicU32::new(0), AtomicU64::new(7)));
        let index = Index::new(vec![counting.clone(), Arc::new(Fixed("fixed", vec![Item::new("x", "Fixed", "apps", 0)]))]);
        assert!(index.refresh(false));
        let generation = index.generation();
        assert!(!index.refresh(true), "nothing changed");
        assert_eq!(index.generation(), generation);
        assert_eq!(titles(&index, "reading"), ["Reading 0"]);
        counting.1.store(8, Ordering::Relaxed);
        assert!(index.refresh(false));
        assert_eq!(titles(&index, "reading"), ["Reading 1"], "read again");
        assert_eq!(titles(&index, "fixed"), ["Fixed"], "the other part kept as it was");
        assert_eq!(counting.0.load(Ordering::Relaxed), 2);
        assert!(index.stats().items == 2 && index.stats().bytes > 0);
    }

    #[test]
    fn answers_come_with_the_index_and_the_last_rows_stay_last() {
        struct Answers;
        impl Provider for Answers {
            fn name(&self) -> &'static str {
                "answers"
            }
            fn answer(&self, query: &Query, _robot: &Robot, out: &mut Vec<Item>) {
                out.push(Item::new("web", format!("Search the web for {}", query.raw), "language", weight::OTHER).ranked(TAIL + 1));
                out.push(Item::new("ask", "Ask", "auto_awesome", weight::OTHER).ranked(TAIL + 2));
                if query.raw == "2+2" {
                    out.push(Item::new("sum", "4", "calculate", weight::OTHER).ranked(0));
                }
            }
            fn open(&self, _item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
                Err(String::new())
            }
        }
        let many: Vec<Item> = (0..50).map(|i| Item::new(format!("{i}"), format!("Item {i}"), "notes", weight::NOTE)).collect();
        let index = Index::new(vec![Arc::new(Fixed("many", many)), Arc::new(Answers)]);
        index.refresh(false);
        let found = titles(&index, "item");
        assert_eq!(found.len(), 22, "twenty and the two last rows");
        assert_eq!(&found[20..], ["Search the web for item".to_owned(), "Ask".to_owned()]);
        assert_eq!(titles(&index, "2+2")[0], "4", "a sum first");
    }
}
