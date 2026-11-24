//! The build log: the album read as an engineering notebook. Photos by day,
//! newest day first, and within a day by where they were taken (the named
//! place, else the event, else the town), each place in the order the day
//! reached it and its photos in the order they were taken, each with its
//! caption, its time and its match.
//!
//! **Export** writes a day, an event, a week or everything as Markdown with
//! its photos, into the album's folder (`Build log/…`), where Catalyst
//! Companion's Files and a computer copying the card both find it. The
//! photos are linked into it, not copied, where the card allows (the same
//! file twice for no space), and copied where it does not.
//!
//! **Send to Team** sends a day as a few short messages (Team carries text,
//! 200 characters a message, no pictures): what the day was, then its
//! first captions.

use std::path::{Path, PathBuf};

use chrono::{NaiveDate, NaiveDateTime};

use super::meta::About;
use crate::photos::Photo;

/// One photo in the log.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub at: NaiveDateTime,
    pub about: About,
}

impl Entry {
    pub fn pixel(&self) -> bool {
        crate::photos::is_pixel(&self.name)
    }
}

/// The photos of a day at one place.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The place line, or empty where nothing is known.
    pub title: String,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub groups: Vec<Group>,
}

impl Day {
    pub fn count(&self) -> usize {
        self.groups.iter().map(|g| g.entries.len()).sum()
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.groups.iter().flat_map(|g| g.entries.iter())
    }

    /// The event the day's photos were at, if any.
    pub fn event(&self) -> Option<&str> {
        self.entries().map(|e| e.about.event.as_str()).find(|e| !e.is_empty())
    }

    /// "sat 21 nov", or with the year when it is not this one.
    pub fn words(&self) -> String {
        let this_year = chrono::Local::now().date_naive().format("%Y").to_string() == self.date.format("%Y").to_string();
        let words = if this_year { self.date.format("%a %-d %b").to_string() } else { self.date.format("%a %-d %b %Y").to_string() };
        words.to_lowercase()
    }
}

/// The album as days, newest first, its photos' sidecars read.
pub fn gather(photos: &[Photo]) -> Vec<Day> {
    let entries: Vec<Entry> = photos.iter().map(|p| Entry { name: p.name.clone(), path: p.path.clone(), at: p.taken, about: super::meta::read(&p.path) }).collect();
    group(entries)
}

/// Entries as days and places.
pub fn group(mut entries: Vec<Entry>) -> Vec<Day> {
    entries.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.name.cmp(&b.name)));
    let mut days: Vec<Day> = Vec::new();
    for e in entries {
        let date = e.at.date();
        let title = e.about.place_line().unwrap_or_default();
        let day = match days.iter_mut().position(|d| d.date == date) {
            Some(i) => &mut days[i],
            None => {
                days.push(Day { date, groups: Vec::new() });
                days.last_mut().expect("just pushed")
            }
        };
        match day.groups.iter_mut().find(|g| g.title == title) {
            Some(g) => g.entries.push(e),
            None => day.groups.push(Group { title, entries: vec![e] }),
        }
    }
    days.sort_by(|a, b| b.date.cmp(&a.date));
    days
}

// ----------------------------------------------------------------- export ---

/// What an export takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Range {
    Day,
    /// Every day at the focused day's event.
    Event,
    /// The seven days to the focused one.
    Week,
    All,
}

/// The days a range takes, from `days`, about the day `at`.
pub fn select(days: &[Day], range: Range, at: NaiveDate) -> Vec<&Day> {
    let event = days.iter().find(|d| d.date == at).and_then(|d| d.event().map(str::to_owned));
    days.iter()
        .filter(|d| match range {
            Range::Day => d.date == at,
            Range::Event => event.as_deref().is_some_and(|e| d.entries().any(|x| x.about.event == e)),
            Range::Week => d.date <= at && (at - d.date).num_days() < 7,
            Range::All => true,
        })
        .collect()
}

/// Text that reads as itself in Markdown.
fn md(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '_' | '[' | ']' | '<' | '>' | '`' | '#' | '|') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The export's folder name: its dates, and the event's name when it is one.
fn folder_name(days: &[&Day], range: Range) -> String {
    let (first, last) = (days.iter().map(|d| d.date).min(), days.iter().map(|d| d.date).max());
    let dates = match (first, last) {
        (Some(a), Some(b)) if a == b => a.format("%Y-%m-%d").to_string(),
        (Some(a), Some(b)) => format!("{} to {}", a.format("%Y-%m-%d"), b.format("%Y-%m-%d")),
        _ => "empty".into(),
    };
    let event = (range == Range::Event).then(|| days.iter().find_map(|d| d.event())).flatten();
    let name = match event {
        Some(e) => format!("{dates} {e}"),
        None => dates,
    };
    name.chars().map(|c| if c.is_alphanumeric() || " -_.,'()".contains(c) { c } else { '_' }).collect::<String>().trim().to_owned()
}

/// The Markdown of `days` (oldest first, as a notebook reads), its images
/// under `images/`.
pub fn markdown(days: &[&Day], heading: &str, h24: bool) -> String {
    let mut out = format!("# {}\n", md(heading));
    let mut days: Vec<&&Day> = days.iter().collect();
    days.sort_by_key(|d| d.date);
    for day in days {
        out.push_str(&format!("\n## {}\n", day.date.format("%A %-d %B %Y")));
        for g in &day.groups {
            out.push_str(&format!("\n### {}\n", if g.title.is_empty() { "Elsewhere".to_owned() } else { md(&g.title) }));
            for e in &g.entries {
                let clock = if h24 { e.at.format("%H:%M").to_string() } else { e.at.format("%-I:%M %P").to_string() };
                let alt = if e.about.caption.is_empty() { e.name.clone() } else { e.about.caption.clone() };
                // A time-lapse is a video: linked, as no Markdown viewer shows one inline.
                if crate::lapse::is_lapse(&e.name) {
                    out.push_str(&format!("\n[Time-lapse: {}](images/{})\n\n", md(&alt), e.name.replace(' ', "%20")));
                } else {
                    out.push_str(&format!("\n![{}](images/{})\n\n", md(&alt), e.name.replace(' ', "%20")));
                }
                let mut line = format!("**{clock}**");
                if !e.about.caption.is_empty() {
                    line.push_str(&format!(" · {}", md(&e.about.caption)));
                    if e.about.by.starts_with("claude") {
                        line.push_str(" *· Claude*");
                    }
                }
                out.push_str(&line);
                out.push('\n');
                let mut also: Vec<String> = Vec::new();
                if !e.about.context.is_empty() {
                    also.push(e.about.context.clone());
                }
                // The event, where the heading is a named place instead.
                if !e.about.event.is_empty() && !g.title.starts_with(&e.about.event) {
                    also.push(e.about.event.clone());
                }
                if !also.is_empty() {
                    out.push_str(&format!("\n*{}*\n", md(&also.join(" · "))));
                }
            }
        }
    }
    out.push_str("\n---\n\nFrom Catalyst Linux's build log. Captions marked Claude were written by Claude Opus 5.5 from the photo; the rest by the team.\n");
    out
}

/// Write an export into `root` (the album's `Build log` folder): its
/// folder, and how many photos it holds.
pub fn export(days: &[&Day], range: Range, root: &Path, h24: bool) -> Result<(PathBuf, usize), String> {
    if days.is_empty() {
        return Err("nothing to export".into());
    }
    let folder = root.join(folder_name(days, range));
    let images = folder.join("images");
    std::fs::create_dir_all(&images).map_err(|_| "the card would not take it".to_owned())?;
    let mut n = 0;
    for e in days.iter().flat_map(|d| d.entries()) {
        let to = images.join(&e.name);
        std::fs::remove_file(&to).ok();
        if std::fs::hard_link(&e.path, &to).is_err() {
            std::fs::copy(&e.path, &to).map_err(|_| format!("{} could not be copied", e.name))?;
        }
        n += 1;
    }
    let heading = match (range, days.iter().find_map(|d| d.event())) {
        (Range::Event, Some(event)) => format!("Build log · {event}"),
        _ if days.len() == 1 => format!("Build log · {}", days[0].date.format("%a %-d %b %Y")),
        _ => {
            let first = days.iter().map(|d| d.date).min().unwrap_or_default();
            let last = days.iter().map(|d| d.date).max().unwrap_or_default();
            format!("Build log · {} to {}", first.format("%-d %b"), last.format("%-d %b %Y"))
        }
    };
    crate::sys::write_atomic(&folder.join("build-log.md"), markdown(days, &heading, h24).as_bytes()).map_err(|_| "the card would not take it".to_owned())?;
    Ok((folder, n))
}

/// Where exports go: the album's own folder, so Companion and a copy of the
/// card find them beside the photos.
pub fn exports_dir() -> PathBuf {
    crate::photos::dir().join("Build log")
}

// --------------------------------------------------------------- the team ---

/// A day as Team's messages: what it was, then its first three captions
/// (each at most Team's 200 characters; four messages at most).
pub fn team_texts(day: &Day, h24: bool) -> Vec<String> {
    let most = crate::team::wire::TEXT_MOST;
    let fit = |s: String| -> String {
        let s: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_owned();
        if s.chars().count() <= most {
            s
        } else {
            format!("{}…", s.chars().take(most - 1).collect::<String>().trim_end())
        }
    };
    let places: Vec<String> = day.groups.iter().filter(|g| !g.title.is_empty()).map(|g| g.title.split(" · ").next().unwrap_or("").to_lowercase()).collect();
    let n = day.count();
    let mut head = format!("build log · {} · {n} photo{}", day.words(), if n == 1 { "" } else { "s" });
    if !places.is_empty() {
        head.push_str(&format!(" · {}", places.join(", ")));
    }
    let mut out = vec![fit(head)];
    // The day's first captions, in the order they were taken.
    let mut captioned: Vec<&Entry> = day.entries().filter(|e| !e.about.caption.is_empty()).collect();
    captioned.sort_by_key(|e| e.at);
    for e in captioned.into_iter().take(3) {
        let clock = if h24 { e.at.format("%H:%M").to_string() } else { e.at.format("%-I:%M%P").to_string() };
        let context = if e.about.context.is_empty() { String::new() } else { format!(" ({})", e.about.context) };
        out.push(fit(format!("{clock}{context} {}", e.about.caption)));
    }
    out
}
