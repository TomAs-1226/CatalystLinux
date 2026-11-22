//! Update logs: what changed in each version of Catalyst Linux.
//!
//! They are written once, in `os/CHANGELOG.md`, a section a version:
//!
//! ```text
//! ## 0.4.1 · 21 Nov 2026
//! ### New
//! - Weather: now, the next 24 hours and the week.
//! ```
//!
//! and read in three places. A release carries its own section in its
//! signed manifest (scripts/release.sh puts it there), so Settings › System
//! update shows what an update brings before it installs. The image carries
//! the whole file (`/usr/share/catalyst/changelog.md`), so About › Update
//! history has every version's notes, offline. And the first start after an
//! update shows that version's notes once, over Home.

use bezel::Rect;
use detent::{role, Animated};

use crate::gfx::{Gfx, ListState, TextSize, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::sysupdate::Status;

/// Where the image keeps the changelog.
const IMAGE_COPY: &str = "/usr/share/catalyst/changelog.md";

/// The update prefs' key for the last version whose notes were shown after
/// an update.
const SHOWN_KEY: &str = "whats-new";

// ----------------------------------------------------------------- notes ---

/// One line of a version's notes.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A `- ` line: one change.
    Bullet(String),
    /// Words that are not a list.
    Text(String),
}

/// The changes under one heading (`### New`), or under none.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: Option<String>,
    pub items: Vec<Item>,
}

/// A version's notes: its groups, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notes {
    pub groups: Vec<Group>,
}

impl Notes {
    /// How many changes: its lines, under every heading.
    pub fn count(&self) -> usize {
        self.groups.iter().map(|g| g.items.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }
}

/// A version, when it came out (if the changelog says), and its notes.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub date: Option<String>,
    pub notes: Notes,
}

/// Markdown's marks that would read as noise on the panel: bold and code.
fn plain(s: &str) -> String {
    s.replace("**", "").replace('`', "").trim().to_owned()
}

/// A heading's words, if the line is one (`### New`).
fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (hashes > 0 && line[hashes..].starts_with(' ')).then(|| line[hashes..].trim())
}

/// A version's notes as written: headings group the lines under them, `- `
/// starts a change, an indented line carries the one above on, and other
/// words are kept as they are. Release notes from before there was a
/// changelog (one plain line) read as a paragraph.
pub fn parse_notes(text: &str) -> Notes {
    let mut groups: Vec<Group> = Vec::new();
    let mut after_blank = true;
    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            after_blank = true;
            continue;
        }
        if let Some(name) = heading(trimmed) {
            let name = plain(name);
            groups.push(Group { name: (!name.is_empty()).then_some(name), items: Vec::new() });
            after_blank = true;
            continue;
        }
        if groups.is_empty() {
            groups.push(Group { name: None, items: Vec::new() });
        }
        let items = &mut groups.last_mut().expect("pushed above").items;
        let indented = trimmed.len() < line.len();
        let bullet = ["- ", "* ", "+ "].iter().find_map(|mark| trimmed.strip_prefix(mark));
        match (bullet, items.last_mut()) {
            (Some(words), _) => items.push(Item::Bullet(plain(words))),
            // A change's words carried onto the next line, indented.
            (None, Some(Item::Bullet(last))) if indented && !after_blank => {
                last.push(' ');
                last.push_str(&plain(trimmed));
            }
            (None, Some(Item::Text(last))) if !after_blank => {
                last.push(' ');
                last.push_str(&plain(trimmed));
            }
            (None, _) => items.push(Item::Text(plain(trimmed))),
        }
        after_blank = false;
    }
    groups.retain(|g| !g.items.is_empty());
    Notes { groups }
}

/// The whole changelog: a release for every `## <version>` section, newest
/// first as written. What comes before the first section is its
/// introduction, for whoever writes it, and is not shown.
pub fn parse(text: &str) -> Vec<Release> {
    let mut out = Vec::new();
    let mut current: Option<(String, Option<String>, String)> = None;
    let finish = |out: &mut Vec<Release>, (version, date, body): (String, Option<String>, String)| {
        out.push(Release { version, date, notes: parse_notes(&body) });
    };
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            if let Some(done) = current.take() {
                finish(&mut out, done);
            }
            let head = head.trim();
            let (version, rest) = head.split_once(char::is_whitespace).unwrap_or((head, ""));
            let date = rest.trim().trim_start_matches(['·', '—', '-', '|']).trim();
            current = Some((version.trim_start_matches('v').to_owned(), (!date.is_empty()).then(|| date.to_owned()), String::new()));
        } else if let Some((_, _, body)) = &mut current {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(done) = current {
        finish(&mut out, done);
    }
    out
}

/// The version without a nightly's date: whose notes it shares.
fn base(version: &str) -> &str {
    version.split('-').next().unwrap_or(version)
}

/// Every version's notes, from the changelog in the image (on a computer,
/// the one in this source tree).
pub fn history() -> Vec<Release> {
    let text = std::fs::read_to_string(IMAGE_COPY).unwrap_or_else(|_| include_str!("../../../../os/CHANGELOG.md").to_owned());
    parse(&text)
}

/// The last version without update logs: a handheld that never marked any
/// notes shown came from it (or before), so everything since is new to it.
const BEFORE_LOGS: &str = "0.4.0";

/// "0.4.2" as numbers, for comparing.
fn numbers(version: &str) -> Vec<u32> {
    base(version).split('.').map(|n| n.parse().unwrap_or(0)).collect()
}

/// The notes new to a handheld now on `running` that last showed those of
/// `shown` (nothing: it came from before update logs): every version after
/// that one, up to the running one, newest first, each with something to say.
fn since_last_shown(all: Vec<Release>, running: &str, shown: &str) -> Vec<Release> {
    let from = numbers(if shown.is_empty() { BEFORE_LOGS } else { shown });
    let to = numbers(running);
    let mut new: Vec<Release> = all.into_iter().filter(|r| !r.notes.is_empty() && numbers(&r.version) > from && numbers(&r.version) <= to).collect();
    new.sort_by_key(|r| std::cmp::Reverse(numbers(&r.version)));
    new
}

/// One version's notes, from the changelog.
pub fn of(version: &str) -> Option<Release> {
    history().into_iter().find(|r| r.version == base(version))
}

/// "1 change", "6 changes".
pub fn changes(n: usize) -> String {
    format!("{n} {}", if n == 1 { "change" } else { "changes" })
}

// ------------------------------------------------------ settings › update ---

/// Whether an update is on its way (found, coming down, or written and
/// waiting for a restart): its own notes, from its manifest, are the ones to show.
fn coming(u: &Status) -> bool {
    matches!(u.state.as_str(), "available" | "downloading" | "installing" | "ready")
}

/// The notes Settings › System update offers: an update's own while one is
/// on its way, otherwise this system's.
fn update_notes(u: &Status) -> Option<(String, Notes)> {
    if coming(u) {
        let notes = parse_notes(&u.notes);
        if !notes.is_empty() {
            return Some((u.version.clone(), notes));
        }
    }
    let running = crate::sysupdate::version();
    of(&running).filter(|r| !r.notes.is_empty()).map(|r| (running, r.notes))
}

/// What the update failing (or not being able to look) has to say instead.
fn note_instead(u: &Status) -> bool {
    !u.detail.is_empty() && u.state != "updated"
}

/// Settings › System update's line about what is new: its name, its value,
/// and whether that reads as "on".
pub fn update_row(u: &Status) -> (String, String, bool) {
    if note_instead(u) {
        return ("Note".into(), u.detail.clone(), false);
    }
    match update_notes(u) {
        Some((version, notes)) => (format!("What's new in {}", base(&version)), format!("{} ›", changes(notes.count())), true),
        None => ("This system".into(), crate::sysupdate::version(), true),
    }
}

/// The page that line opens, if it opens one.
pub fn update_page(u: &Status) -> Option<WhatsNew> {
    if note_instead(u) {
        return None;
    }
    let (version, notes) = update_notes(u)?;
    let ours = coming(u) && version == u.version;
    let (then, caption) = match u.state.as_str() {
        "available" if ours => (Then::Install, format!("available · {}", crate::sysupdate::megabytes(u.size)).trim_end_matches(" · ").to_owned()),
        "ready" if ours => (Then::Restart, "ready · restart to finish".to_owned()),
        _ if ours => (Then::Back, crate::sysupdate::summary(u)),
        _ => (Then::Back, "this system".to_owned()),
    };
    let release = Release { version, date: None, notes };
    Some(WhatsNew::single(release, caption, then))
}

// ---------------------------------------------------------- after update ---

/// The version whose notes are due now, once, after an update to it.
pub fn due_after_update(u: &Status) -> Option<String> {
    due(u, &crate::sysupdate::version(), &crate::sysupdate::pref(SHOWN_KEY, ""))
}

/// An update to the running version has just been kept (`updated`), and its
/// notes have not been shown since. A first start, a flashed card, or an
/// update that did not take never qualify.
fn due(u: &Status, running: &str, shown: &str) -> Option<String> {
    (u.state == "updated" && u.version == running && shown != running).then(|| running.to_owned())
}

/// Shown (or there was nothing to show): not again for this version.
pub fn mark_shown(version: &str) {
    crate::sysupdate::set_pref(SHOWN_KEY, version);
}

// ----------------------------------------------------------- what's new ---

/// What Enter does on a page of notes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Then {
    /// Download and install the update the notes are for.
    Install,
    /// Restart into the update, written and waiting.
    Restart,
    /// Read after an update: done.
    Done,
    /// Only reading.
    Back,
}

/// A version's notes, to read: the version large, then its changes on
/// tonal cards by heading, scrolled with F and X (Pg Up and Pg Dn a page at
/// a time). In the update history, Z and C turn to the next version.
pub struct WhatsNew {
    title: &'static str,
    releases: Vec<Release>,
    at: usize,
    /// The line beside the version, for a page about one release.
    caption: Option<String>,
    then: Then,
    history: bool,
    running: String,
    /// Where it is scrolled to (pixels from the top), and where it is going.
    scroll: Animated,
    target: f32,
    /// A push past either end: it gives a little and springs back.
    bump: f32,
    placed: bool,
    /// What the last layout allowed: the furthest it scrolls, a step, a page.
    max: f32,
    step: f32,
    page: f32,
    /// Turning to another version: it slides in from the side it came from.
    slide: Animated,
    laid: Option<Laid>,
}

impl WhatsNew {
    fn new(title: &'static str, releases: Vec<Release>, at: usize, caption: Option<String>, then: Then, history: bool) -> Self {
        WhatsNew {
            title,
            releases,
            at,
            caption,
            then,
            history,
            running: base(&crate::sysupdate::version()).to_owned(),
            scroll: Animated::new(0.0, role::hold()).with_rest(0.1),
            target: 0.0,
            bump: 0.0,
            placed: false,
            max: 0.0,
            step: 34.0,
            page: 100.0,
            slide: Animated::new(0.0, role::page()).with_rest(0.004),
            laid: None,
        }
    }

    fn single(release: Release, caption: String, then: Then) -> Self {
        Self::new("What's new", vec![release], 0, Some(caption), then, false)
    }

    /// The notes of the version just updated to, shown once after it starts,
    /// and of any it skipped on the way (←→ turn to them), newest first.
    pub fn after_update(version: &str) -> Option<Self> {
        let releases = since_last_shown(history(), version, &crate::sysupdate::pref(SHOWN_KEY, ""));
        match releases.len() {
            0 => None,
            1 => Some(Self::single(releases.into_iter().next()?, "now on this handheld".into(), Then::Done)),
            _ => Some(Self::new("What's new", releases, 0, None, Then::Done, true)),
        }
    }

    /// The update history, open at one version.
    pub fn history(releases: Vec<Release>, at: usize) -> Self {
        let at = at.min(releases.len().saturating_sub(1));
        Self::new("Notes", releases, at, None, Then::Back, true)
    }

    fn release(&self) -> Option<&Release> {
        self.releases.get(self.at)
    }

    /// The line beside the version: what was given, or (in the history)
    /// whether it is this system and when it came out.
    fn caption_of(&self, r: &Release) -> String {
        if let Some(c) = &self.caption {
            return c.clone();
        }
        let date = r.date.as_deref().map(str::to_lowercase);
        match (r.version == self.running, date) {
            (true, Some(d)) => format!("this system · {d}"),
            (true, None) => "this system".into(),
            (false, Some(d)) => d,
            (false, None) => String::new(),
        }
    }

    fn scroll_by(&mut self, by: f32) {
        let want = (self.target + by).clamp(0.0, self.max);
        if want == self.target && by != 0.0 {
            self.bump = by.signum();
        }
        self.target = want;
    }

    /// To the next version (+1, older) or the last (−1, newer).
    fn turn(&mut self, by: isize) {
        let to = self.at as isize + by;
        if to < 0 || to >= self.releases.len() as isize {
            self.bump = by.signum() as f32;
            return;
        }
        self.at = to as usize;
        self.laid = None;
        self.target = 0.0;
        self.scroll.jump(0.0);
        self.slide.jump(by as f32);
        self.slide.set(0.0);
    }

    fn hint(&self) -> &'static str {
        match self.then {
            Then::Install => "enter: install · esc: back",
            Then::Restart => "enter: restart to update · esc: back",
            Then::Done if self.history && self.releases.len() > 1 => "←→: the versions before · enter: done",
            Then::Done => "enter: done",
            Then::Back if self.history && self.releases.len() > 1 => "←→: another version · esc: back",
            Then::Back => "esc: back",
        }
    }
}

/// The notes laid out at one text size: the version's header, then a card
/// for each heading with its lines, all in page pixels from the top.
struct Laid {
    key: (usize, TextSize, crate::look::Theme),
    hero_h: f32,
    cards: Vec<Card>,
    height: f32,
}

struct Card {
    y: f32,
    h: f32,
    name: Option<String>,
    /// Each line: where its middle is (from the card's top), whether it
    /// starts a change (a dot before it), its words, and how far in it sits.
    lines: Vec<(f32, bool, String, f32)>,
}

/// The cards' width: the page, less the scroll marker's lane.
fn card_w() -> f32 {
    W - 2.0 * PAD - 6.0
}

/// How far in a change's words sit (after its dot), and a paragraph's.
const BULLET_IN: f32 = 21.0;
const TEXT_IN: f32 = 11.0;

fn pitch(g: &Gfx) -> f32 {
    g.ty.body.size + 4.0
}

fn hero_h(g: &Gfx) -> f32 {
    (g.ty.display(24.0).size + 12.0).round()
}

fn lay_out(g: &mut Gfx, notes: &Notes) -> (Vec<Card>, f32) {
    let ty = g.ty;
    let line_h = pitch(g);
    let mut cards = Vec::new();
    let mut y = hero_h(g) + 2.0;
    for group in &notes.groups {
        let mut lines = Vec::new();
        let mut at = 8.0;
        if group.name.is_some() {
            at += ty.caption.size + 6.0;
        }
        for item in &group.items {
            let (words, dot, inset) = match item {
                Item::Bullet(w) => (w, true, BULLET_IN),
                Item::Text(w) => (w, false, TEXT_IN),
            };
            for (k, line) in crate::help::wrap(g, words, ty.body, card_w() - inset - 10.0).into_iter().enumerate() {
                lines.push((at + line_h * 0.5, dot && k == 0, line, inset));
                at += line_h;
            }
            at += 2.0;
        }
        let h = (at + 5.0).round();
        cards.push(Card { y, h, name: group.name.clone(), lines });
        y += h + crate::gfx::GAP;
    }
    (cards, y - crate::gfx::GAP)
}

fn page_bottom(g: &Gfx) -> f32 {
    H - g.hint_h() - 2.0
}

/// How much of a line centred at `cy` shows between `top` and `bottom`:
/// whole while it is in, fading over its last half-line as it leaves.
fn edge_fade(cy: f32, top: f32, bottom: f32, line_h: f32) -> f32 {
    let half = line_h * 0.5;
    ((cy - top) / half).min((bottom - cy) / half).clamp(0.0, 1.0)
}

impl Screen for WhatsNew {
    fn title(&self) -> &str {
        self.title
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => {
                let version = self.release().map(|r| r.version.clone()).unwrap_or_default();
                let started = match self.then {
                    Then::Install => crate::sysupdate::run("install").map(|()| format!("downloading {version}")),
                    Then::Restart => crate::sysupdate::run("apply").map(|()| format!("restarting into {version}")),
                    Then::Done | Then::Back => return Nav::Pop,
                };
                *cx.say = Some(started.unwrap_or_else(|why| why));
                return Nav::Pop;
            }
            Key::Up => self.scroll_by(-self.step),
            Key::Down | Key::Char(' ') => self.scroll_by(self.step),
            Key::PageUp => self.scroll_by(-self.page),
            Key::PageDown => self.scroll_by(self.page),
            Key::Home => self.scroll_by(-self.target),
            Key::End => self.scroll_by(self.max - self.target),
            Key::Left | Key::Right if self.history => self.turn(if key == Key::Left { -1 } else { 1 }),
            Key::Left => return Nav::Pop,
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.tick(dt) | self.slide.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let Some(release) = self.release().cloned() else {
            g.empty("history", "no notes for this version");
            return;
        };
        let key = (self.at, g.text_size, g.theme);
        if self.laid.as_ref().is_none_or(|l| l.key != key) {
            let (cards, height) = lay_out(g, &release.notes);
            self.laid = Some(Laid { key, hero_h: hero_h(g), cards, height });
        }
        let laid = self.laid.as_ref().expect("laid out above");
        let (top, bottom) = (g.top(), page_bottom(g));
        let view = bottom - top;
        self.max = (laid.height - view).max(0.0);
        self.step = pitch(g) * 2.0;
        self.page = (view - pitch(g)).max(pitch(g));
        self.target = self.target.clamp(0.0, self.max);
        if !self.placed || g.calm {
            self.scroll.jump(self.target);
            self.placed = true;
        } else if self.scroll.target() != self.target {
            self.scroll.set(self.target);
            g.started_moving();
        }
        let bump = std::mem::take(&mut self.bump);
        if bump != 0.0 && !g.calm {
            self.scroll.nudge(bump * 260.0);
            g.started_moving();
        }
        if g.calm {
            self.slide.jump(0.0);
        }
        let s = self.scroll.value();
        let ox = g.ox;
        g.ox += self.slide.value() * 18.0;
        g.canvas.set_clip(Rect::new(0.0, top - 1.0, W, view + 1.0));

        // The version, large, and beside it what it is and how much changed.
        if let Some(rise) = g.rise(0.0) {
            let y = top - s + rise;
            let display = ty.display(24.0);
            let cy = y + laid.hero_h * 0.5 - 1.0;
            // Scrolled away, it fades as it goes under the title.
            let alpha = edge_fade(cy, top, bottom, laid.hero_h * 0.8);
            let shown = base(&release.version).to_owned();
            let version_w = g.measure(&shown, display);
            g.text_alpha(PAD + 2.0, cy, &shown, display, t.ink, alpha);
            let right = PAD + card_w();
            let room = right - (PAD + 2.0 + version_w) - 12.0;
            let mut caption = self.caption_of(&release);
            if release.version.contains('-') {
                caption = format!("{} · {caption}", release.version.split_once('-').map_or("", |(_, rest)| rest)).trim_end_matches(" · ").to_owned();
            }
            let count = g.text.fit(&changes(release.notes.count()), &ty.caption, room);
            let gap = 3.0;
            let (c1, c2) = (cy - (ty.caption.size + gap) * 0.5, cy + (ty.caption.size + gap) * 0.5);
            let right_aligned = |g: &mut Gfx, cy: f32, words: &str, colour: bezel::Color| {
                let w = g.measure(words, ty.caption);
                g.text_alpha(right - w, cy, words, ty.caption, colour, alpha);
            };
            if caption.is_empty() {
                right_aligned(g, cy, &count, t.dim);
            } else {
                let fitted = g.text.fit(&caption, &ty.caption, room);
                right_aligned(g, c1, &fitted, t.dim);
                right_aligned(g, c2, &count, t.faint.mix(t.dim, 0.5));
            }
        }

        // The changes, a card for each heading.
        let radius = g.tile_radius().min(14.0);
        let ink = t.ink.mix(t.dim, 0.2);
        for (i, card) in laid.cards.iter().enumerate() {
            let y = top - s + card.y;
            if y > bottom || y + card.h < top {
                continue;
            }
            let Some(rise) = g.rise(1.0 + i as f32) else { continue };
            let r = Rect::new(PAD, y + rise, card_w(), card.h);
            g.surface(r, radius, t.surface1);
            if let Some(name) = &card.name {
                let cy = r.y + 7.0 + ty.caption.size * 0.5;
                let alpha = edge_fade(cy, top, bottom, ty.caption.size + 6.0);
                g.text_alpha(r.x + TEXT_IN, cy, &name.to_lowercase(), ty.caption, t.dim, alpha);
            }
            for (ly, dot, words, inset) in &card.lines {
                let cy = r.y + ly;
                // A line part-way out of view fades as it goes, rather than
                // being cut through its letters.
                let alpha = edge_fade(cy, top, bottom, pitch(g));
                if alpha <= 0.0 {
                    continue;
                }
                if *dot {
                    g.canvas.circle(r.x + 12.5 + g.ox, cy, 1.9, t.dim, alpha);
                }
                g.text_alpha(r.x + inset, cy, words, ty.body, ink, alpha);
            }
        }
        g.canvas.clear_clip();
        g.ox = ox;

        // Where it is in a page longer than the panel.
        if laid.height > view {
            let track = view;
            let thumb = (track * view / laid.height).max(10.0);
            let y = top + (track - thumb) * (s / self.max.max(1.0)).clamp(0.0, 1.0);
            let faint = t.faint;
            g.surface(Rect::new(W - 5.0, y, 2.5, thumb), 1.25, faint);
        }
        g.hint(self.hint());
    }
}

// --------------------------------------------------------------- history ---

/// About › Update history: every version, newest first, each opening its notes.
pub struct History {
    releases: Vec<Release>,
    running: String,
    list: ListState,
}

impl History {
    pub fn new() -> Self {
        History { releases: history(), running: base(&crate::sysupdate::version()).to_owned(), list: ListState::default() }
    }

    fn value(&self, r: &Release) -> String {
        let when = if r.version == self.running { "this system".to_owned() } else { r.date.clone().map(|d| d.to_lowercase()).unwrap_or_default() };
        format!("{when} ›").trim_start().to_owned()
    }
}

impl Screen for History {
    fn title(&self) -> &str {
        // "Update history" would run into the island.
        "History"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Left => Nav::Pop,
            Key::Enter | Key::Right if !self.releases.is_empty() => Nav::Push(Box::new(WhatsNew::history(self.releases.clone(), self.list.focus))),
            other => {
                crate::screens::list_key(&mut self.list, other, self.releases.len());
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if self.releases.is_empty() {
            g.empty("history", "no update history in this system");
            return;
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), H);
        for (i, y) in self.list.layout(g, self.releases.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let release = &self.releases[i];
            let value = self.value(release);
            let ink = if release.version == self.running { t.ink } else { t.dim };
            g.row(r, &release.version, &value, ink, i == self.list.focus);
        }
        self.list.scrollbar(g, self.releases.len(), top, bottom - 2.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# What's new\n\nAn introduction, not shown.\n\n## 0.4.1\n\n### New\n- Weather: now, the hours\n  and the week.\n- **Update logs**, in `About`.\n\n### Fixed\n- Sending setup works.\n\n## v0.4.0 · 20 Nov 2026\n- Battery saver.\n\nA paragraph\nover two lines.\n\n## 0.3.0 — 19 Nov 2026\n### New\n- Recent apps.\n### Empty\n";

    #[test]
    fn a_changelog_reads_as_versions_headings_and_changes() {
        let all = parse(SAMPLE);
        assert_eq!(all.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(), ["0.4.1", "0.4.0", "0.3.0"]);
        assert_eq!(all.iter().map(|r| r.date.as_deref()).collect::<Vec<_>>(), [None, Some("20 Nov 2026"), Some("19 Nov 2026")]);
        let newest = &all[0].notes;
        assert_eq!(newest.groups.len(), 2);
        assert_eq!(newest.groups[0].name.as_deref(), Some("New"));
        assert_eq!(
            newest.groups[0].items,
            [Item::Bullet("Weather: now, the hours and the week.".into()), Item::Bullet("Update logs, in About.".into())],
            "an indented line carries its change on; bold and code marks go"
        );
        assert_eq!(newest.count(), 3);
        // No heading: one group without a name; a paragraph kept whole.
        let older = &all[1].notes;
        assert_eq!(older.groups.len(), 1);
        assert_eq!(older.groups[0].name, None);
        assert_eq!(older.groups[0].items, [Item::Bullet("Battery saver.".into()), Item::Text("A paragraph over two lines.".into())]);
        // A heading with nothing under it is left out.
        assert_eq!(all[2].notes.groups.len(), 1);
    }

    #[test]
    fn notes_from_before_the_changelog_read_as_a_line() {
        let notes = parse_notes("faster glass, favourites");
        assert_eq!(notes.groups, [Group { name: None, items: vec![Item::Text("faster glass, favourites".into())] }]);
        assert!(parse_notes("").is_empty());
        assert!(parse_notes("\n\n### Nothing\n").is_empty());
    }

    #[test]
    fn the_projects_changelog_has_this_version_and_reads_cleanly() {
        let all = history();
        assert!(!all.is_empty());
        let running = base(&crate::sysupdate::version()).to_owned();
        assert!(all.iter().any(|r| r.version == running), "os/CHANGELOG.md has a section for {running}");
        for r in &all {
            assert!(!r.notes.is_empty(), "{} has notes", r.version);
            assert!(r.version.chars().next().is_some_and(|c| c.is_ascii_digit()), "{} is a version", r.version);
        }
        // Newest first: each version above the one before it.
        let numbers: Vec<Vec<u32>> = all.iter().map(|r| r.version.split('.').filter_map(|p| p.parse().ok()).collect()).collect();
        assert!(numbers.windows(2).all(|w| w[0] > w[1]), "{numbers:?}");
    }

    #[test]
    fn after_an_update_the_versions_skipped_on_the_way_come_too() {
        let all = parse("## 0.4.3\n- c\n\n## 0.4.2\n- b\n\n## 0.4.1\n- a\n\n## 0.4.0\n- old\n\n## 0.3.0\n- older\n");
        let versions = |r: Vec<Release>| r.into_iter().map(|r| r.version).collect::<Vec<_>>();
        // From 0.4.0, which had no update logs: both since, newest first.
        assert_eq!(versions(since_last_shown(all.clone(), "0.4.2", "")), ["0.4.2", "0.4.1"]);
        // From one whose notes were shown: only what came after it.
        assert_eq!(versions(since_last_shown(all.clone(), "0.4.3", "0.4.1")), ["0.4.3", "0.4.2"]);
        assert_eq!(versions(since_last_shown(all.clone(), "0.4.2", "0.4.1")), ["0.4.2"]);
        // A nightly shares its base version's notes.
        assert_eq!(versions(since_last_shown(all, "0.4.2-nightly.20261121.0400", "0.4.1")), ["0.4.2"]);
    }

    #[test]
    fn whats_new_shows_once_after_an_update_and_never_otherwise() {
        let s = |state: &str, version: &str| Status { state: state.into(), version: version.into(), ..Status::default() };
        assert_eq!(due(&s("updated", "0.4.1"), "0.4.1", "").as_deref(), Some("0.4.1"));
        assert_eq!(due(&s("updated", "0.4.1"), "0.4.1", "0.4.1"), None, "once");
        assert_eq!(due(&s("updated", "0.4.1"), "0.4.1", "0.4.0").as_deref(), Some("0.4.1"), "the last shown was an older one");
        assert_eq!(due(&s("", ""), "0.4.1", ""), None, "a first start");
        assert_eq!(due(&s("current", "0.4.1"), "0.4.1", ""), None, "a flashed card");
        assert_eq!(due(&s("trying", "0.4.1"), "0.4.1", ""), None, "not until it is kept");
        assert_eq!(due(&s("failed", "0.4.1"), "0.4.0", ""), None, "an update that did not take");
    }

    #[test]
    fn settings_offers_an_update_s_own_notes_while_it_is_on_its_way() {
        crate::shell::tests::sandbox();
        let mut u = Status { state: "available".into(), version: "9.0.0".into(), size: 12_400_000, notes: "### New\n- One.\n- Two.".into(), ..Status::default() };
        assert_eq!(update_row(&u), ("What's new in 9.0.0".into(), "2 changes ›".into(), true));
        let page = update_page(&u).expect("a page");
        assert_eq!((page.then, page.caption.as_deref()), (Then::Install, Some("available · 12 mb")));
        u.state = "ready".into();
        assert_eq!(update_page(&u).unwrap().then, Then::Restart);
        // A failure's note comes first, and opens nothing.
        u.state = "failed".into();
        u.detail = "9.0.0 did not start".into();
        assert_eq!(update_row(&u).0, "Note");
        assert!(update_page(&u).is_none());
        // Nothing on its way: this system's own notes, from the changelog.
        let current = Status { state: "current".into(), version: crate::sysupdate::version(), ..Status::default() };
        let (name, value, _) = update_row(&current);
        assert_eq!(name, format!("What's new in {}", base(&crate::sysupdate::version())));
        assert!(value.ends_with(" changes ›") || value == "1 change ›", "{value}");
        assert_eq!(update_page(&current).unwrap().then, Then::Back);
    }

    #[test]
    fn every_line_of_every_version_fits_at_every_size_and_look() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for theme in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
                g.theme = theme;
                g.set_text_size(size);
                let ty = g.ty;
                for release in history() {
                    let (cards, height) = lay_out(&mut g, &release.notes);
                    assert!(height > hero_h(&g));
                    for card in &cards {
                        for (_, _, words, inset) in &card.lines {
                            let w = g.measure(words, ty.body);
                            assert!(w <= card_w() - inset - 10.0 + 0.5, "{theme:?} {size:?} {}: {words} is {w}", release.version);
                        }
                        let last = card.lines.last().map_or(0.0, |l| l.0);
                        assert!(last + pitch(&g) * 0.5 <= card.h, "the card holds its last line");
                    }
                    // The version beside its captions.
                    let version_w = g.measure(&release.version, ty.display(24.0));
                    let caption_w = ["this system · 20 nov 2026", "available · 12 mb", "ready · restart to finish"].iter().map(|c| g.measure(c, ty.caption)).fold(0.0, f32::max);
                    assert!(PAD + 2.0 + version_w + 12.0 + caption_w <= PAD + card_w(), "{theme:?} {size:?}: {} and its caption", release.version);
                }
                // Every title whole beside an island showing a robot's name.
                let room = (W - 70.0) * 0.5 - PAD - 8.0;
                for title in ["What's new", "Notes", "History"] {
                    let title = if theme == crate::look::Theme::Bezel { title.to_lowercase() } else { title.to_owned() };
                    assert!(g.measure(&title, ty.title) <= room, "{theme:?} {size:?}: {title}");
                }
                // Every hint whole, and the page's top line under the island.
                for then in [Then::Install, Then::Restart, Then::Done, Then::Back] {
                    let hint = WhatsNew { then, ..WhatsNew::history(history(), 0) }.hint();
                    assert!(g.measure(hint, ty.caption) <= W - 2.0 * PAD, "{size:?}: {hint}");
                }
                assert!(page_bottom(&g) - g.top() >= hero_h(&g) + 2.0 * pitch(&g), "{size:?}: room for the header and a line");
            }
        }
    }

    fn press(page: &mut dyn Screen, key: Key) -> Nav {
        let mut robot = Robot::none();
        let mut settings = crate::shell::Settings::default();
        let mut say = None;
        page.key(key, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true })
    }

    #[test]
    fn the_notes_scroll_within_their_length_and_the_history_turns() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        g.calm = true;
        let all = history();
        let mut page = WhatsNew::history(all.clone(), 0);
        page.draw(&mut g, &Robot::none());
        assert!(page.max > 0.0, "the newest notes and the header are longer than the panel");
        for _ in 0..200 {
            press(&mut page, Key::Down);
        }
        assert_eq!(page.target, page.max, "no further than the end");
        press(&mut page, Key::Home);
        assert_eq!(page.target, 0.0);
        // C (right) turns to the next, older version, back at its top.
        press(&mut page, Key::End);
        press(&mut page, Key::Right);
        assert_eq!((page.at, page.target), (1, 0.0));
        press(&mut page, Key::Left);
        press(&mut page, Key::Left);
        assert_eq!(page.at, 0, "and no newer than the newest");
        assert!(matches!(press(&mut page, Key::Back), Nav::Pop));
        // A page read after an update: Enter is done.
        let mut after = WhatsNew::after_update(&all[0].version).expect("the newest has notes");
        assert!(matches!(press(&mut after, Key::Enter), Nav::Pop));
        assert!(WhatsNew::after_update("0.0.1").is_none(), "a version with no notes shows nothing");
    }

    #[test]
    fn the_history_lists_every_version_and_opens_each() {
        let mut list = History::new();
        assert_eq!(list.releases.len(), history().len());
        for r in &list.releases {
            let value = list.value(r);
            assert!(value.ends_with('›'), "{value}");
            assert_eq!(value.starts_with("this system"), r.version == list.running, "{}: {value}", r.version);
        }
        press(&mut list, Key::Down);
        assert!(matches!(press(&mut list, Key::Enter), Nav::Push(_)));
    }
}
