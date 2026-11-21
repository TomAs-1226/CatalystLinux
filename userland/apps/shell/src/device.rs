//! The handheld's own name and its time zone, chosen in setup or in
//! Settings › Name & time.
//!
//! Both are kept on the data partition beside the other settings, since the
//! system itself is read-only. `hostname` holds the name: catalyst-late
//! gives it to the kernel at boot, and DHCP tells the network it (a router
//! lists the handheld by it). `timezone` holds the zone's name, and
//! `localtime` beside it is a link to that zone's rules, which
//! `/etc/localtime` points at, so every program keeps the same local time.
//! The clock sets itself from the network when one with a way out is joined
//! (catalyst-time, run after DHCP), and the RX8130 clock chip keeps it while
//! the handheld is off.

use std::path::Path;

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

/// The name the image gives every handheld.
pub const DEFAULT_NAME: &str = "catalyst";

/// The longest name kept: a hostname may be 63, but this one is read on a
/// 320-pixel screen and in a router's list.
pub const NAME_MOST: usize = 24;

/// The zone a handheld starts in, until one is chosen.
pub const DEFAULT_ZONE: &str = "Etc/UTC";

/// Where each zone's rules are, on the handheld.
const ZONEINFO: &str = "/usr/share/zoneinfo";

/// Written by catalyst-time each time the network set the clock: when.
const SYNCED: &str = "/run/catalyst/time-synced";

// ------------------------------------------------------------------ name ---

/// A character the name may be typed with: letters, digits, a hyphen, and a
/// space (which becomes a hyphen).
pub fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == ' ' || c == '_'
}

/// A key typed into the name, as the network will take it: lower case, a
/// hyphen for a space, no hyphen first or twice. Anything else is let go.
pub fn type_name(name: &mut String, c: char) {
    if !name_char(c) || name.chars().count() >= NAME_MOST {
        return;
    }
    let c = if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' };
    if c == '-' && (name.is_empty() || name.ends_with('-')) {
        return;
    }
    name.push(c);
}

/// A name as a network takes one: lower case, letters, digits and single
/// hyphens, neither starting nor ending with one, at most `NAME_MOST` long.
pub fn clean_name(typed: &str) -> String {
    let mut name = String::new();
    for c in typed.chars() {
        let c = match c {
            ' ' | '_' | '-' => '-',
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase(),
            _ => continue,
        };
        if c == '-' && (name.is_empty() || name.ends_with('-')) {
            continue;
        }
        name.push(c);
    }
    name.truncate(NAME_MOST);
    name.trim_end_matches('-').to_owned()
}

/// The handheld's name: the one chosen, else the kernel's (on the handheld),
/// else the image's own.
pub fn name() -> String {
    read_name(&crate::config_dir())
}

fn read_name(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("hostname"))
        .ok()
        .map(|n| clean_name(n.trim()))
        .filter(|n| !n.is_empty())
        .or_else(|| crate::sys::on_handheld().then(crate::sys::hostname).flatten())
        .unwrap_or_else(|| DEFAULT_NAME.to_owned())
}

/// Give the handheld a name: kept for the next boot and, on the handheld,
/// the kernel's from now on. The name as kept, or why not.
pub fn set_name(typed: &str) -> Result<String, String> {
    write_name(&crate::config_dir(), typed)
}

fn write_name(dir: &Path, typed: &str) -> Result<String, String> {
    let name = clean_name(typed);
    if name.is_empty() {
        return Err("a name needs a letter or a digit".into());
    }
    std::fs::create_dir_all(dir).ok();
    crate::sys::write_atomic(&dir.join("hostname"),format!("{name}\n").as_bytes()).map_err(|_| "could not keep the name".to_owned())?;
    if crate::sys::on_handheld() {
        std::fs::write("/proc/sys/kernel/hostname", &name).map_err(|_| "kept for the next start".to_owned())?;
    }
    Ok(name)
}

/// A name to offer in setup: the one in use, or the image's own with the
/// team after it, once a team is known.
pub fn suggested_name(team: &str) -> String {
    suggest(&name(), team)
}

fn suggest(current: &str, team: &str) -> String {
    match team.trim().parse::<u32>() {
        Ok(team) if current == DEFAULT_NAME => format!("{DEFAULT_NAME}-{team}"),
        _ => current.to_owned(),
    }
}

// ----------------------------------------------------------------- zones ---

/// A time zone offered by name: its id in the zone database, the place it
/// is known by, its part of the world, and its standard offset from UTC in
/// minutes (summer time, where kept, adds an hour).
pub struct Zone {
    pub id: &'static str,
    pub city: &'static str,
    pub region: &'static str,
    pub offset: i16,
}

const fn z(id: &'static str, city: &'static str, region: &'static str, offset: i16) -> Zone {
    Zone { id, city, region, offset }
}

const AMERICAS: &str = "americas";
const EUROPE: &str = "europe";
const AFRICA: &str = "africa and the middle east";
const ASIA: &str = "asia";
const PACIFIC: &str = "australia and the pacific";
const NONE: &str = "no time zone";

/// The zones the picker lists, by part of the world, west to east; most
/// FRC teams' and events' among them. Search finds every other zone the
/// handheld knows as well.
pub const ZONES: &[Zone] = &[
    z("Pacific/Honolulu", "Honolulu · Hawaii", AMERICAS, -600),
    z("America/Anchorage", "Anchorage · Alaska", AMERICAS, -540),
    z("America/Los_Angeles", "Los Angeles · Pacific", AMERICAS, -480),
    z("America/Vancouver", "Vancouver", AMERICAS, -480),
    z("America/Tijuana", "Tijuana", AMERICAS, -480),
    z("America/Phoenix", "Phoenix · Arizona", AMERICAS, -420),
    z("America/Denver", "Denver · Mountain", AMERICAS, -420),
    z("America/Edmonton", "Edmonton", AMERICAS, -420),
    z("America/Chicago", "Chicago · Central", AMERICAS, -360),
    z("America/Winnipeg", "Winnipeg", AMERICAS, -360),
    z("America/Mexico_City", "Mexico City", AMERICAS, -360),
    z("America/New_York", "New York · Eastern", AMERICAS, -300),
    z("America/Detroit", "Detroit", AMERICAS, -300),
    z("America/Toronto", "Toronto", AMERICAS, -300),
    z("America/Bogota", "Bogotá", AMERICAS, -300),
    z("America/Lima", "Lima", AMERICAS, -300),
    z("America/Halifax", "Halifax · Atlantic", AMERICAS, -240),
    z("America/Puerto_Rico", "Puerto Rico", AMERICAS, -240),
    z("America/Santiago", "Santiago", AMERICAS, -240),
    z("America/St_Johns", "St. John's", AMERICAS, -210),
    z("America/Sao_Paulo", "São Paulo", AMERICAS, -180),
    z("America/Argentina/Buenos_Aires", "Buenos Aires", AMERICAS, -180),
    z("Europe/London", "London", EUROPE, 0),
    z("Europe/Dublin", "Dublin", EUROPE, 0),
    z("Europe/Lisbon", "Lisbon", EUROPE, 0),
    z("Europe/Paris", "Paris", EUROPE, 60),
    z("Europe/Amsterdam", "Amsterdam", EUROPE, 60),
    z("Europe/Berlin", "Berlin", EUROPE, 60),
    z("Europe/Madrid", "Madrid", EUROPE, 60),
    z("Europe/Rome", "Rome", EUROPE, 60),
    z("Europe/Zurich", "Zürich", EUROPE, 60),
    z("Europe/Stockholm", "Stockholm", EUROPE, 60),
    z("Europe/Warsaw", "Warsaw", EUROPE, 60),
    z("Europe/Athens", "Athens", EUROPE, 120),
    z("Europe/Helsinki", "Helsinki", EUROPE, 120),
    z("Europe/Kyiv", "Kyiv", EUROPE, 120),
    z("Europe/Istanbul", "Istanbul", EUROPE, 180),
    z("Europe/Moscow", "Moscow", EUROPE, 180),
    z("Africa/Lagos", "Lagos", AFRICA, 60),
    z("Africa/Cairo", "Cairo", AFRICA, 120),
    z("Africa/Johannesburg", "Johannesburg", AFRICA, 120),
    z("Asia/Jerusalem", "Jerusalem", AFRICA, 120),
    z("Africa/Nairobi", "Nairobi", AFRICA, 180),
    z("Asia/Riyadh", "Riyadh", AFRICA, 180),
    z("Asia/Dubai", "Dubai", AFRICA, 240),
    z("Asia/Karachi", "Karachi", ASIA, 300),
    z("Asia/Kolkata", "India", ASIA, 330),
    z("Asia/Kathmandu", "Kathmandu", ASIA, 345),
    z("Asia/Dhaka", "Dhaka", ASIA, 360),
    z("Asia/Bangkok", "Bangkok", ASIA, 420),
    z("Asia/Jakarta", "Jakarta", ASIA, 420),
    z("Asia/Ho_Chi_Minh", "Ho Chi Minh", ASIA, 420),
    z("Asia/Shanghai", "China", ASIA, 480),
    z("Asia/Hong_Kong", "Hong Kong", ASIA, 480),
    z("Asia/Taipei", "Taipei", ASIA, 480),
    z("Asia/Singapore", "Singapore", ASIA, 480),
    z("Asia/Manila", "Manila", ASIA, 480),
    z("Asia/Seoul", "Seoul", ASIA, 540),
    z("Asia/Tokyo", "Tokyo", ASIA, 540),
    z("Australia/Perth", "Perth", PACIFIC, 480),
    z("Australia/Adelaide", "Adelaide", PACIFIC, 570),
    z("Australia/Darwin", "Darwin", PACIFIC, 570),
    z("Australia/Brisbane", "Brisbane", PACIFIC, 600),
    z("Australia/Sydney", "Sydney", PACIFIC, 600),
    z("Australia/Melbourne", "Melbourne", PACIFIC, 600),
    z("Pacific/Auckland", "Auckland", PACIFIC, 720),
    z("Etc/UTC", "UTC", NONE, 0),
];

/// The zone in use: the one chosen, else UTC.
pub fn zone() -> String {
    read_zone(&crate::config_dir())
}

fn read_zone(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("timezone"))
        .ok()
        .map(|z| z.trim().to_owned())
        .filter(|z| valid_zone_id(z))
        .unwrap_or_else(|| DEFAULT_ZONE.to_owned())
}

/// A zone id as the database writes them: `Region/City`, letters, digits,
/// `_`, `-`, `+` and slashes, no `..`.
fn valid_zone_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('/')
        && !id.contains("..")
        && id.chars().all(|c| c.is_ascii_alphanumeric() || "/_-+".contains(c))
}

/// The place a zone is known by: the listed name, else the last part of its id.
pub fn zone_city(id: &str) -> String {
    match ZONES.iter().find(|z| z.id == id) {
        Some(z) => z.city.to_owned(),
        None => id.rsplit('/').next().unwrap_or(id).replace('_', " "),
    }
}

/// A standard offset as it is usually written: `UTC`, `UTC+1`, `UTC−5`, `UTC+5:30`.
pub fn offset_label(minutes: i16) -> String {
    if minutes == 0 {
        return "UTC".into();
    }
    let sign = if minutes < 0 { '−' } else { '+' };
    let (h, m) = (minutes.unsigned_abs() / 60, minutes.unsigned_abs() % 60);
    if m == 0 {
        format!("UTC{sign}{h}")
    } else {
        format!("UTC{sign}{h}:{m:02}")
    }
}

/// A zone as a row shows it, opening the picker: the place without the
/// zone's own name after it, and its offset ("New York · UTC−5 ›"; UTC once).
pub fn zone_label(id: &str) -> String {
    let city = zone_city(id);
    let place = city.split(" · ").next().unwrap_or(&city).to_owned();
    match zone_offset(id) {
        Some(offset) if offset != place => format!("{place} · {offset} ›"),
        _ => format!("{place} ›"),
    }
}

/// A listed zone's offset, written out; nothing for a zone only search found.
pub fn zone_offset(id: &str) -> Option<String> {
    ZONES.iter().find(|z| z.id == id).map(|z| offset_label(z.offset))
}

/// Use `id` from now on: kept for the next start, and on the handheld the
/// system's local time at once (this program's clocks, and what it starts).
pub fn set_zone(id: &str) -> Result<(), String> {
    write_zone(&crate::config_dir(), id)?;
    // Weather's tile follows the zone (its units, its city).
    crate::weather::tile_changed();
    Ok(())
}

fn write_zone(dir: &Path, id: &str) -> Result<(), String> {
    if !valid_zone_id(id) {
        return Err("not a time zone".into());
    }
    let rules = Path::new(ZONEINFO).join(id);
    let here = crate::sys::on_handheld();
    if here && !rules.is_file() {
        return Err(format!("no rules for {id} here"));
    }
    std::fs::create_dir_all(dir).ok();
    crate::sys::write_atomic(&dir.join("timezone"), format!("{id}\n").as_bytes()).map_err(|_| "could not keep the time zone".to_owned())?;
    if here {
        link_localtime(dir, &rules).map_err(|_| "could not set the time zone".to_owned())?;
        follow_zone(id);
    }
    Ok(())
}

/// On the handheld, at start: the chosen zone in use, its link in place.
pub fn apply_saved() {
    if !crate::sys::on_handheld() {
        return;
    }
    let id = zone();
    let rules = Path::new(ZONEINFO).join(&id);
    if rules.is_file() {
        let dir = crate::config_dir();
        if std::fs::read_link(dir.join("localtime")).ok().as_deref() != Some(rules.as_path()) {
            link_localtime(&dir, &rules).ok();
        }
        follow_zone(&id);
    }
}

/// Point `localtime` (where `/etc/localtime` leads) at a zone's rules, in one step.
#[cfg(unix)]
fn link_localtime(dir: &Path, rules: &Path) -> std::io::Result<()> {
    let link = dir.join("localtime");
    let fresh = dir.join(".localtime.new");
    std::fs::remove_file(&fresh).ok();
    std::os::unix::fs::symlink(rules, &fresh)?;
    std::fs::rename(&fresh, &link)
}

#[cfg(not(unix))]
fn link_localtime(_dir: &Path, _rules: &Path) -> std::io::Result<()> {
    Ok(())
}

/// This program's clocks follow the zone, and so does what it starts. (The
/// local time is cached by the path of `/etc/localtime`, which never changes
/// here; the zone named in TZ is looked at again when it does.)
fn follow_zone(id: &str) {
    std::env::set_var("TZ", id);
}

/// Every zone the handheld has rules for, by id, from the database's own
/// list (none on a computer).
pub fn known_zones() -> Vec<String> {
    let mut ids: Vec<String> = ["zone1970.tab", "zone.tab"]
        .iter()
        .filter_map(|f| std::fs::read_to_string(Path::new(ZONEINFO).join(f)).ok())
        .flat_map(|text| text.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split('\t').nth(2).map(str::to_owned)).collect::<Vec<_>>())
        .filter(|id| valid_zone_id(id))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Where the clock's time came from, in a few words, for Settings and setup.
pub fn clock_note() -> String {
    if !crate::sys::on_handheld() {
        return "this computer".into();
    }
    let synced = std::fs::read_to_string(SYNCED).ok().and_then(|t| t.trim().parse::<i64>().ok());
    match synced {
        Some(at) => network_note((chrono::Utc::now().timestamp() - at).max(0)),
        None if Path::new("/dev/rtc0").exists() => "clock chip".into(),
        None => "not set yet".into(),
    }
}

/// Ask the network for the time now, in the background (catalyst-time: the
/// time servers, else a web answer's date); what the island says meanwhile.
pub fn set_clock_now() -> String {
    if !crate::sys::on_handheld() {
        return "the clock is this computer's".into();
    }
    match std::process::Command::new("sh").args(["-c", "/usr/sbin/catalyst-time sync now >/dev/null 2>&1 &"]).status() {
        Ok(_) => "asking the network for the time".into(),
        Err(_) => "could not ask for the time".into(),
    }
}

/// The network set the clock `ago` seconds ago.
fn network_note(ago: i64) -> String {
    match ago {
        a if a < 90 => "network, just now".into(),
        a if a < 3600 => format!("network, {} min ago", a / 60),
        a if a < 86_400 => format!("network, {} h ago", a / 3600),
        _ => "network".into(),
    }
}

// ---------------------------------------------------------------- picker ---

/// A line of the picker: a part of the world, or a zone in it.
#[derive(Clone, PartialEq, Debug)]
enum Line {
    Head(&'static str),
    Listed(usize),
    /// A zone only search found: one the database has, not in the list.
    Other(String),
}

impl Line {
    fn id(&self) -> Option<&str> {
        match self {
            Line::Head(_) => None,
            Line::Listed(i) => Some(ZONES[*i].id),
            Line::Other(id) => Some(id),
        }
    }
}

/// The time zone picker: the zones by part of the world, the one in use
/// marked; Space or / searches every zone the handheld knows.
pub struct ZonePicker {
    list: ListState,
    lines: Vec<Line>,
    search: Option<Field>,
    known: Vec<String>,
    chosen: String,
}

/// The longest search typed.
const SEARCH_MOST: usize = 24;

impl ZonePicker {
    pub fn new() -> Self {
        Self::on(zone())
    }

    /// The picker with `chosen` marked as the zone in use.
    fn on(chosen: String) -> Self {
        let mut picker = ZonePicker { list: ListState::default(), lines: Vec::new(), search: None, known: known_zones(), chosen };
        picker.fill();
        // Opened on the zone in use.
        picker.list.focus = picker.lines.iter().position(|l| l.id() == Some(picker.chosen.as_str())).unwrap_or(1);
        picker
    }

    /// The lines for the search as it stands: every listed zone under its
    /// part of the world, or only those the search finds.
    fn fill(&mut self) {
        let query = self.search.as_ref().map(|f| f.text.trim().to_lowercase()).unwrap_or_default();
        self.lines.clear();
        if query.is_empty() {
            let mut region = "";
            for (i, zone) in ZONES.iter().enumerate() {
                if zone.region != region {
                    region = zone.region;
                    self.lines.push(Line::Head(region));
                }
                self.lines.push(Line::Listed(i));
            }
            return;
        }
        let words: Vec<&str> = query.split_whitespace().collect();
        let finds = |hay: &str| {
            let hay = hay.to_lowercase().replace('_', " ");
            words.iter().all(|w| hay.contains(w))
        };
        for (i, zone) in ZONES.iter().enumerate() {
            if finds(&format!("{} {} {}", zone.city, zone.id, zone.region)) {
                self.lines.push(Line::Listed(i));
            }
        }
        for id in &self.known {
            if self.lines.len() >= 40 {
                break;
            }
            if !ZONES.iter().any(|z| z.id == id) && finds(id) {
                self.lines.push(Line::Other(id.clone()));
            }
        }
    }

    fn is_head(&self, i: usize) -> bool {
        matches!(self.lines.get(i), Some(Line::Head(_)))
    }

    /// The lines filled again for the search, the focus on the first zone.
    fn refill(&mut self) {
        self.fill();
        self.list = ListState::default();
        self.list.focus = usize::from(self.is_head(0));
    }

    fn step(&mut self, key: Key) {
        let len = self.lines.len();
        let heads: Vec<bool> = (0..len).map(|i| self.is_head(i)).collect();
        crate::tools::step_over_headings(&mut self.list, key, len, |i| heads[i]);
    }

    /// Left and Right jump a part of the world at a time.
    fn jump(&mut self, forward: bool) {
        let heads: Vec<usize> = (0..self.lines.len()).filter(|i| self.is_head(*i)).collect();
        let at = self.list.focus;
        let target = if forward { heads.iter().find(|h| **h > at).copied() } else { heads.iter().rev().find(|h| **h + 1 < at).copied().or(heads.first().copied()) };
        if let Some(h) = target {
            self.list.focus = (h + 1).min(self.lines.len().saturating_sub(1));
        }
    }

    fn choose(&mut self, cx: &mut Cx) -> Nav {
        let Some(id) = self.lines.get(self.list.focus).and_then(Line::id).map(str::to_owned) else { return Nav::Stay };
        match set_zone(&id) {
            Ok(()) => {
                *cx.say = Some(format!("time zone: {}", zone_city(&id).to_lowercase()));
                Nav::Pop
            }
            Err(why) => {
                *cx.say = Some(why);
                Nav::Stay
            }
        }
    }
}

impl Default for ZonePicker {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for ZonePicker {
    fn title(&self) -> &str {
        "Time zone"
    }

    fn wants_text(&self) -> bool {
        self.search.is_some()
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.search {
            match key {
                Key::Up | Key::Down | Key::PageUp | Key::PageDown => self.step(key),
                Key::Enter => return self.choose(cx),
                other => match field.key(other, |c| c.is_ascii_alphanumeric() || " /_-".contains(c)) {
                    Typed::Cancelled => {
                        self.search = None;
                        self.fill();
                        self.list.focus = self.lines.iter().position(|l| l.id() == Some(self.chosen.as_str())).unwrap_or(1);
                    }
                    Typed::Done(_) => return self.choose(cx),
                    Typed::More => self.refill(),
                },
            }
            return Nav::Stay;
        }
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => return self.choose(cx),
            Key::Char(' ') | Key::Char('/') => {
                self.search = Some(Field::new("search for a city or a zone", "", SEARCH_MOST));
                self.refill();
            }
            Key::Left => self.jump(false),
            Key::Right => self.jump(true),
            other => self.step(other),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let row_h = g.row_h();
        let mut top = g.top();
        let hint_room = g.hint_h() + 3.0;
        if let Some(field) = &self.search {
            field.draw(g, "enter chooses · esc: the whole list");
            top += ty.caption.size + ty.label.size + 20.0 + 5.0;
        } else {
            g.hint("space searches · enter chooses");
        }
        let bottom = H - hint_room;
        if self.lines.is_empty() {
            g.text_at(W * 0.5, (top + bottom) * 0.5, Align::Center, "no zone by that name", ty.body, t.dim);
            return;
        }
        let len = self.lines.len();
        for (i, y) in self.list.layout(g, len, top, row_h, 3.0, bottom) {
            match &self.lines[i] {
                Line::Head(name) => crate::tools::heading(g, y, row_h, name),
                line => {
                    let id = line.id().unwrap_or_default().to_owned();
                    let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
                    let focused = i == self.list.focus;
                    let offset = zone_offset(&id).unwrap_or_else(|| id.split('/').next().unwrap_or_default().to_lowercase());
                    let in_use = id == self.chosen;
                    let value = if in_use { format!("in use · {offset}") } else { offset };
                    g.row(r, &zone_city(&id), &value, if in_use { t.ink } else { t.dim }, focused);
                }
            }
        }
        self.list.scrollbar(g, len, top, bottom - 2.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_made_fit_for_a_network() {
        assert_eq!(clean_name("Thomas's Cardputer"), "thomass-cardputer");
        assert_eq!(clean_name("  pit  crew__2 "), "pit-crew-2");
        assert_eq!(clean_name("--5805--"), "5805");
        assert_eq!(clean_name("!!!"), "");
        assert_eq!(clean_name(&"a".repeat(40)).len(), NAME_MOST);
        assert!(!clean_name("a-very-long-name-that-ends-on-a-hyphen-").ends_with('-'));
    }

    /// A folder of its own for one test (they run side by side).
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("catalyst-device-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_name_is_kept_and_suggested_with_the_team() {
        let dir = scratch("name");
        assert_eq!(read_name(&dir), DEFAULT_NAME);
        assert_eq!(suggest(DEFAULT_NAME, "5805"), "catalyst-5805");
        assert_eq!(suggest(DEFAULT_NAME, ""), "catalyst");
        assert_eq!(write_name(&dir, "Pit Crew").as_deref(), Ok("pit-crew"));
        assert_eq!(read_name(&dir), "pit-crew");
        assert_eq!(suggest(&read_name(&dir), "5805"), "pit-crew", "a name already chosen is kept");
        assert!(write_name(&dir, "?!").is_err());
        assert_eq!(read_name(&dir), "pit-crew", "and a bad one changes nothing");
    }

    #[test]
    fn a_zone_is_kept_and_bad_ones_refused() {
        let dir = scratch("zone");
        assert_eq!(read_zone(&dir), DEFAULT_ZONE);
        assert!(write_zone(&dir, "America/New_York").is_ok());
        assert_eq!(read_zone(&dir), "America/New_York");
        assert!(write_zone(&dir, "../../etc/passwd").is_err());
        assert!(write_zone(&dir, "").is_err());
        assert_eq!(read_zone(&dir), "America/New_York");
        assert_eq!(zone_city("America/New_York"), "New York · Eastern");
        assert_eq!(zone_city("America/Indiana/Indianapolis"), "Indianapolis");
        assert_eq!(zone_label("America/New_York"), "New York · UTC−5 ›");
        assert_eq!(zone_label("Etc/UTC"), "UTC ›", "UTC said once");
        assert_eq!(zone_label("America/Indiana/Indianapolis"), "Indianapolis ›");
    }

    #[test]
    fn the_clock_says_when_the_network_last_set_it() {
        assert_eq!(network_note(30), "network, just now");
        assert_eq!(network_note(59 * 60 + 30), "network, 59 min ago");
        assert_eq!(network_note(5 * 3600), "network, 5 h ago");
        assert_eq!(network_note(3 * 86_400), "network");
    }

    #[test]
    fn offsets_are_written_as_people_write_them() {
        assert_eq!(offset_label(0), "UTC");
        assert_eq!(offset_label(-300), "UTC−5");
        assert_eq!(offset_label(330), "UTC+5:30");
        assert_eq!(offset_label(-210), "UTC−3:30");
        assert_eq!(offset_label(345), "UTC+5:45");
    }

    #[test]
    fn every_listed_zone_is_unique_and_in_order() {
        let mut seen = std::collections::HashSet::new();
        for zone in ZONES {
            assert!(seen.insert(zone.id), "{} twice", zone.id);
            assert!(valid_zone_id(zone.id), "{}", zone.id);
        }
        // Grouped: a region, once left, does not come back.
        let mut regions: Vec<&str> = Vec::new();
        for zone in ZONES {
            if regions.last() != Some(&zone.region) {
                assert!(!regions.contains(&zone.region), "{} split", zone.region);
                regions.push(zone.region);
            }
        }
    }

    #[test]
    fn the_picker_searches_and_steps_over_its_headings() {
        let mut picker = ZonePicker::on(DEFAULT_ZONE.to_owned());
        assert_eq!(picker.lines[picker.list.focus].id(), Some(DEFAULT_ZONE), "opened on the zone in use");
        assert!(matches!(picker.lines[0], Line::Head(_)));
        picker.list.focus = 1;
        for _ in 0..picker.lines.len() {
            picker.step(Key::Down);
            assert!(!picker.is_head(picker.list.focus));
        }
        // Search opened: still the whole list, the focus on a zone, not a heading.
        picker.search = Some(Field::new("search", "", SEARCH_MOST));
        picker.refill();
        assert!(!picker.is_head(picker.list.focus));
        picker.search = Some(Field::new("search", "york", SEARCH_MOST));
        picker.fill();
        assert_eq!(picker.lines, vec![Line::Listed(ZONES.iter().position(|z| z.id == "America/New_York").unwrap())]);
        picker.search = Some(Field::new("search", "sao paulo", SEARCH_MOST));
        picker.fill();
        assert_eq!(picker.lines.len(), 1, "found by its id without the accent typed: {:?}", picker.lines);
        picker.search = Some(Field::new("search", "zzz", SEARCH_MOST));
        picker.fill();
        assert!(picker.lines.is_empty());
        picker.search = Some(Field::new("search", "pacific", SEARCH_MOST));
        picker.fill();
        assert!(picker.lines.len() >= 3, "by region too: {:?}", picker.lines);
    }

    #[test]
    fn every_zone_row_fits_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            for zone in ZONES {
                let value = format!("in use · {}", offset_label(zone.offset));
                let w = g.measure(zone.city, ty.body) + 22.0 + g.measure(&value, ty.label) + 10.0;
                assert!(w <= W - 2.0 * PAD - 5.0, "{} crowds its row at {size:?}: {w:.0}", zone.city);
            }
        }
    }
}
