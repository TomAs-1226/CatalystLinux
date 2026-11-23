//! Share: records to another handheld or a phone, with no internet. A
//! match's records or the whole event's as QR codes, shown one after
//! another; codes read with the camera; and a CSV file for Catalyst
//! Companion to download through Files.

use std::time::{Duration, Instant};

use bezel::Rect;

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Match,
    Event,
    Read,
    Csv,
}

const ROWS: [Row; 4] = [Row::Match, Row::Event, Row::Read, Row::Csv];

pub struct ShareMenu {
    event: String,
    /// The matches with records, in order, and the one chosen.
    matches: Vec<(String, String, usize)>,
    chosen: usize,
    records: usize,
    codes: usize,
    csv: Option<String>,
    list: ListState,
}

impl ShareMenu {
    pub fn new(event: String, _next: Option<usize>) -> Self {
        let mut me = ShareMenu { event, matches: Vec::new(), chosen: 0, records: 0, codes: 0, csv: None, list: ListState::default() };
        me.reload();
        me.chosen = me.matches.len().saturating_sub(1);
        me
    }

    fn reload(&mut self) {
        let form = super::form();
        let event = self.event.clone();
        let (matches, records, codes) = super::with_store(|s| {
            let mut matches: Vec<(String, String, usize)> = Vec::new();
            for r in s.of_event(&event) {
                match matches.iter_mut().find(|m| m.0 == r.match_key) {
                    Some(m) => m.2 += 1,
                    None => matches.push((r.match_key.clone(), r.label.clone(), 1)),
                }
            }
            let all: Vec<&super::store::Record> = s.of_event(&event).collect();
            let codes = if all.is_empty() { 0 } else { super::share::chunks(&form, &event, &all).len() };
            (matches, all.len(), codes)
        });
        self.matches = matches;
        self.chosen = self.chosen.min(self.matches.len().saturating_sub(1));
        self.records = records;
        self.codes = codes;
    }

    fn codes_of(&self, match_key: Option<&str>) -> Vec<String> {
        let form = super::form();
        super::with_store(|s| {
            let picked: Vec<&super::store::Record> = s.of_event(&self.event).filter(|r| match_key.is_none_or(|k| r.match_key == k)).collect();
            super::share::chunks(&form, &self.event, &picked)
        })
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Screen for ShareMenu {
    fn title(&self) -> &str {
        "Share"
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn resume(&mut self) {
        self.reload();
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        let row = ROWS[self.list.focus.min(ROWS.len() - 1)];
        vec![match row {
            Row::Match => keys("a match", &[("enter", "Show its codes"), ("t", "Send to the team"), ("← →", "Another match")]),
            Row::Event => keys("the event", &[("enter", "Show its codes"), ("t", "Send to the team")]),
            Row::Read => keys("read codes", &[("enter", "Open the camera")]),
            Row::Csv => keys("csv file", &[("enter", "Write it for Files")]),
        }]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let row = ROWS[self.list.focus.min(ROWS.len() - 1)];
        match key {
            Key::Back => return Nav::Pop,
            Key::Left | Key::Right if row == Row::Match && !self.matches.is_empty() => {
                let n = self.matches.len() as i32;
                self.chosen = (self.chosen as i32 + if key == Key::Left { -1 } else { 1 }).rem_euclid(n) as usize;
            }
            // Straight to the team, over Team, rather than as codes.
            Key::Char('t') if matches!(row, Row::Match | Row::Event) => {
                let (label, texts) = match (row, self.matches.get(self.chosen)) {
                    (Row::Match, Some((key, label, _))) => (format!("Scout {label}"), self.codes_of(Some(key))),
                    (Row::Event, _) if self.records > 0 => (format!("Scout {}", self.event), self.codes_of(None)),
                    _ => (String::new(), Vec::new()),
                };
                *cx.say = Some(match crate::team::share(crate::team::wire::ShareKind::Scout, &label, &texts) {
                    _ if texts.is_empty() => "nothing scouted yet".into(),
                    Ok(_) => "sent to the team".into(),
                    Err(why) => why,
                });
            }
            Key::Enter => match row {
                Row::Match => match self.matches.get(self.chosen) {
                    Some((key, label, _)) => return Nav::Push(Box::new(Codes::new(label.clone(), self.codes_of(Some(key))))),
                    None => *cx.say = Some("nothing scouted yet".into()),
                },
                Row::Event if self.records > 0 => return Nav::Push(Box::new(Codes::new("Event".into(), self.codes_of(None)))),
                Row::Event => *cx.say = Some("nothing scouted yet".into()),
                Row::Read => return Nav::Push(Box::new(crate::camera::Camera::scanning())),
                Row::Csv => {
                    let form = super::form();
                    let event = self.event.clone();
                    let written = super::with_store(|s| {
                        let all: Vec<&super::store::Record> = s.of_event(&event).collect();
                        super::share::export_csv(&super::dir(), &form, &event, &all)
                    });
                    match written {
                        Ok(path) => {
                            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            *cx.say = Some(format!("written: scout/export/{name}"));
                            self.csv = Some(name);
                        }
                        Err(_) => *cx.say = Some("the card would not take the file".into()),
                    }
                }
            },
            other => crate::screens::list_key(&mut self.list, other, ROWS.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let top = g.top();
        let bottom = g.above_hint();
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, ROWS.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name, value) = match ROWS[i] {
                Row::Match => match self.matches.get(self.chosen) {
                    Some((_, label, n)) => (format!("Match {label} as codes"), plural(*n, "record", "records")),
                    None => ("A match as codes".into(), "none scouted".into()),
                },
                Row::Event => ("Whole event as codes".into(), if self.records == 0 { "none scouted".into() } else { plural(self.codes, "code", "codes") }),
                Row::Read => ("Read codes with the camera".into(), super::inbox_words().map(|(p, _)| p).unwrap_or_default()),
                Row::Csv => ("CSV file for Companion".into(), self.csv.clone().unwrap_or_else(|| plural(self.records, "row", "rows"))),
            };
            g.row(r, &name, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, ROWS.len(), top, bottom);
        let row = ROWS[self.list.focus.min(ROWS.len() - 1)];
        g.hint_any(match row {
            Row::Match => &["enter: codes · t: to the team · ←→: match", "enter: show the codes · ←→: another match", "enter: show · ←→: match", "enter: show"],
            Row::Read => &["enter: the camera, codes import as they are read", "enter: open the camera", "enter: camera"],
            Row::Csv => &["enter: write it to scout/export for Files", "enter: write it", "enter: write"],
            Row::Event => &["enter: show the codes · t: send to the team", "enter: show the codes, one after another", "enter: show the codes", "enter: show"],
        });
    }
}

// ------------------------------------------------------------------- codes ---

/// How long each code shows: long enough for a camera that looks five
/// times a second to read it twice.
const EVERY: Duration = Duration::from_millis(900);

/// Codes shown one after another, round and round, at a pace a camera
/// keeps up with. Space holds one; Left and Right step.
pub struct Codes {
    title: String,
    texts: Vec<String>,
    qrs: Vec<Option<qrcodegen::QrCode>>,
    at: usize,
    paused: bool,
    since: Instant,
}

impl Codes {
    pub fn new(title: String, texts: Vec<String>) -> Self {
        let qrs = texts.iter().map(|_| None).collect();
        Codes { title, texts, qrs, at: 0, paused: false, since: Instant::now() }
    }

    fn qr(&mut self, i: usize) -> Option<&qrcodegen::QrCode> {
        if self.qrs.get(i)?.is_none() {
            self.qrs[i] = qrcodegen::QrCode::encode_text(&self.texts[i], qrcodegen::QrCodeEcc::Low).ok();
        }
        self.qrs[i].as_ref()
    }

    fn step(&mut self, by: i32) {
        let n = self.texts.len().max(1) as i32;
        self.at = (self.at as i32 + by).rem_euclid(n) as usize;
        self.since = Instant::now();
    }

    fn cycling(&self) -> bool {
        !self.paused && self.texts.len() > 1
    }
}

impl Screen for Codes {
    fn title(&self) -> &str {
        &self.title
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn keeps_awake(&self) -> bool {
        true
    }

    /// The next code is due: looked in on at its pace, never on the frame beat.
    fn ambient(&self) -> Option<Duration> {
        self.cycling().then_some(EVERY / 3)
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        if self.cycling() && self.since.elapsed() >= EVERY {
            self.step(1);
            return true;
        }
        false
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.paused {
            vec![keys("codes", &[("space", "Go on"), ("← →", "Last, next code")])]
        } else {
            vec![keys("codes", &[("space", "Hold this one"), ("← →", "Last, next code")])]
        }
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Char(' ') => {
                self.paused = !self.paused;
                self.since = Instant::now();
            }
            Key::Left => self.step(-1),
            Key::Right => self.step(1),
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        if self.texts.is_empty() {
            g.empty("qr_code_2", "nothing scouted to share yet");
            return;
        }
        let at = self.at;
        let side = H - top - 4.0;
        let room = Rect::new(PAD, top, side, side);
        let Some(qr) = self.qr(at).cloned() else { return };
        let code = crate::share::draw_qr(g, &qr, room);
        let x = code.right() + 12.0;
        let width = W - PAD - x;
        // Which code this is, large, and how far round.
        let n = self.texts.len();
        let big = ty.display(30.0);
        let baseline = code.y + 6.0 + big.size * 0.78;
        let w = g.text(x, baseline, &format!("{}", at + 1), big, t.ink);
        g.text(x + w + 5.0, baseline, &format!("of {n}"), ty.label, t.dim);
        g.meter(Rect::new(x, baseline + 9.0, width, 4.0), (at + 1) as f32 / n as f32, t.ice);
        let records = self.texts[at].lines().count().saturating_sub(1);
        let line1 = baseline + 20.0 + ty.caption.size * 0.5;
        g.text_fit(x, line1, &plural(records, "record in it", "records in it"), ty.caption, t.dim, width);
        // The keys, a line each, along the foot of the code.
        let lines: &[&str] = match (n, self.paused) {
            (1, _) => &["Scout's camera", "or a phone reads it"],
            (_, true) => &["held", "space: go on", "←→: step"],
            (_, false) => &["space: hold one", "←→: step"],
        };
        let foot = code.bottom() - 4.0 - ty.caption.size * 0.5;
        let line_h = ty.caption.size + 3.0;
        for (k, line) in lines.iter().rev().enumerate() {
            g.text_fit(x, foot - k as f32 * line_h, line, ty.caption, t.dim, width);
        }
    }
}
