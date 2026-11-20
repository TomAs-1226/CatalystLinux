//! Trends across matches: from every pulled log's digest, how low the
//! battery went, the brownouts, the loop overruns, the hottest motor and the
//! busiest CAN bus, match by match, as small bar charts with the newest
//! match at the right. Anything that is worse in the newest match than in
//! the ones before it is flagged, so a robot slowly going wrong over an
//! event shows before it fails.
//!
//! Opened from the top of Logs. Up and Down choose a measure; Left and Right
//! go back through the matches; Enter opens the match chosen.

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::logs::{Digest, Row};
use crate::platform::Key;
use crate::robot::Robot;
use crate::screens::{list_key, LogDetail};
use crate::shell::{Cx, Nav, Screen};

/// The most matches drawn: an event's qualifying, near enough.
const MOST: usize = 16;
/// The room a match's bar takes, and so the width of the bars: the name and
/// the value have the rest of the row.
const SLOT: f32 = 7.0;
const BARS_W: f32 = MOST as f32 * SLOT;
/// How many matches before the newest it is compared with.
const AGAINST: usize = 5;

/// One measure, read from a match's digest.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Metric {
    Battery,
    Brownouts,
    Overruns,
    Hottest,
    Can,
}

impl Metric {
    pub const ALL: [Metric; 5] = [Metric::Battery, Metric::Brownouts, Metric::Overruns, Metric::Hottest, Metric::Can];

    pub fn name(self) -> &'static str {
        match self {
            Metric::Battery => "Battery low",
            Metric::Brownouts => "Brownouts",
            Metric::Overruns => "Overruns",
            Metric::Hottest => "Motor heat",
            Metric::Can => "CAN peak",
        }
    }

    /// The few words a worsening one is reported in.
    pub fn worse_words(self) -> &'static str {
        match self {
            Metric::Battery => "battery sagging lower",
            Metric::Brownouts => "brownouts",
            Metric::Overruns => "loop overrunning more",
            Metric::Hottest => "motors running hotter",
            Metric::Can => "CAN busier",
        }
    }

    pub fn of(self, d: &Digest) -> Option<f64> {
        match self {
            Metric::Battery => d.battery_low,
            Metric::Brownouts => d.brownouts.map(f64::from),
            Metric::Overruns => d.loop_overruns.map(f64::from),
            Metric::Hottest => d.hottest.as_ref().map(|(_, t)| *t),
            Metric::Can => d.can_peak,
        }
    }

    pub fn show(self, v: f64) -> String {
        match self {
            Metric::Battery => format!("{v:.1} v"),
            Metric::Brownouts | Metric::Overruns => format!("{v:.0}"),
            Metric::Hottest => format!("{v:.0} °c"),
            Metric::Can => format!("{:.0}%", v * 100.0),
        }
    }

    /// How tall a bar is, 0 to 1, among values up to `peak`. The scale has a
    /// floor so that one quiet match does not make every bar full.
    fn height(self, v: f64, peak: f64) -> f32 {
        let h = match self {
            // From 6 v (well into brownout) to a full 13.
            Metric::Battery => (v - 6.0) / 7.0,
            Metric::Brownouts => v / peak.max(2.0),
            Metric::Overruns => v / peak.max(5.0),
            // From room temperature up to 90 °c, or the hottest seen.
            Metric::Hottest => (v - 20.0) / (peak.max(90.0) - 20.0),
            Metric::Can => v,
        };
        h.clamp(0.0, 1.0) as f32
    }

    /// Whether `latest` is worse than the matches before it (`before`, the
    /// nearest last), beyond what match-to-match noise explains.
    pub fn worse(self, latest: f64, before: &[f64]) -> bool {
        if before.is_empty() {
            return false;
        }
        let mean = before.iter().sum::<f64>() / before.len() as f64;
        match self {
            Metric::Battery => latest < mean - 0.4 || latest < 9.0,
            Metric::Brownouts => latest > 0.0 && latest > mean,
            Metric::Overruns => latest > mean + 3.0,
            Metric::Hottest => latest > mean + 6.0 || latest >= 80.0,
            Metric::Can => latest > mean + 0.08 || latest >= 0.9,
        }
    }
}

/// The matches that have been read, oldest first, the last `MOST` of them:
/// (title, the log's row).
fn matches(rows: &[Row]) -> Vec<&Row> {
    let mut read: Vec<&Row> = rows.iter().filter(|r| r.digest.is_some()).collect();
    read.reverse();
    let from = read.len().saturating_sub(MOST);
    read.split_off(from)
}

/// Whether there is anything to compare: two matches read.
pub fn available(rows: &[Row]) -> bool {
    rows.iter().filter(|r| r.digest.is_some()).count() >= 2
}

/// The measures worse in the newest match than in those before it.
pub fn worsening(rows: &[Row]) -> Vec<Metric> {
    let read = matches(rows);
    let Some((latest, before)) = read.split_last() else { return Vec::new() };
    Metric::ALL
        .into_iter()
        .filter(|m| {
            let Some(now) = m.of(latest.digest.as_ref().expect("read")) else { return false };
            let earlier: Vec<f64> = before.iter().rev().take(AGAINST).filter_map(|r| m.of(r.digest.as_ref().expect("read"))).collect();
            m.worse(now, &earlier)
        })
        .collect()
}

/// The line Logs' trends row carries: what is getting worse, or that nothing is.
pub fn headline(rows: &[Row]) -> (bool, String) {
    let worse = worsening(rows);
    match worse.as_slice() {
        [] => (false, "nothing getting worse".into()),
        [one] => (true, one.worse_words().into()),
        [first, rest @ ..] => (true, format!("{} · {} more", first.worse_words(), rest.len())),
    }
}

pub struct Trends {
    list: ListState,
    /// The match chosen, counted back from the newest (0).
    back: usize,
}

impl Trends {
    pub fn new() -> Self {
        Trends { list: ListState::default(), back: 0 }
    }
}

impl Screen for Trends {
    fn title(&self) -> &str {
        "Trends"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let rows = cx.robot.logs.list();
        let read = matches(&rows);
        match key {
            Key::Back => return Nav::Pop,
            Key::Left if self.back + 1 < read.len() => self.back += 1,
            Key::Right if self.back > 0 => self.back -= 1,
            Key::Enter => {
                if let Some(detail) = read.len().checked_sub(self.back + 1).and_then(|i| LogDetail::of(read[i])) {
                    return Nav::Push(Box::new(detail));
                }
            }
            other => list_key(&mut self.list, other, Metric::ALL.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let rows = robot.logs.list();
        let read = matches(&rows);
        if read.len() < 2 {
            g.empty("trending_up", "trends need two matches read");
            return;
        }
        self.back = self.back.min(read.len() - 1);
        let chosen = read.len() - 1 - self.back;
        let digests: Vec<&Digest> = read.iter().map(|r| r.digest.as_ref().expect("read")).collect();
        let worse = worsening(&rows);
        let (top, row_h) = (g.top(), g.row_h());
        let bottom = H - PAD - g.hint_h();
        for (i, y) in self.list.layout(g, Metric::ALL.len(), top, row_h, 3.0, bottom + 2.0) {
            let metric = Metric::ALL[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            g.row(r, "", "", t.ink, focused);
            let flagged = worse.contains(&metric) && self.back == 0;
            // The value at the match chosen; at the newest, worse ones marked.
            let value = metric.of(digests[chosen]).map_or("–".into(), |v| metric.show(v));
            let value = if flagged { format!("{} {value}", if metric == Metric::Battery { "▼" } else { "▲" }) } else { value };
            let bars = Rect::new(r.right() - 8.0 - BARS_W, r.y + 4.0, BARS_W, r.h - 8.0);
            let value_w = g.text_at(bars.x - 8.0, r.cy(), Align::Right, &value, ty.label, if flagged { t.amber_ink } else { t.ink });
            let (name, style) = name_and_style(g, metric);
            g.text_fit(r.x + 10.0, r.cy(), &name, style, t.ink, bars.x - 8.0 - value_w - 6.0 - (r.x + 10.0));
            // One bar a match, the newest at the right.
            let peak = digests.iter().filter_map(|d| metric.of(d)).fold(0.0, f64::max);
            let bar_w = SLOT - 2.0;
            for (k, d) in digests.iter().enumerate() {
                let x = bars.right() - (digests.len() - k) as f32 * SLOT + 1.0 + g.ox;
                let ink = match (k == chosen, k == digests.len() - 1) {
                    (_, true) if flagged => t.amber_ink,
                    (true, _) => t.ink,
                    (false, true) => t.ice,
                    _ => t.faint,
                };
                match metric.of(d) {
                    Some(v) => {
                        let h = (metric.height(v, peak) * bars.h).max(1.5);
                        g.canvas.rrect(Rect::new(x, bars.bottom() - h, bar_w, h), 1.0, ink, 1.0);
                    }
                    // Not in that match's digest: a mark where it would stand.
                    None => g.canvas.rrect(Rect::new(x, bars.bottom() - 1.0, bar_w, 1.0), 0.5, t.faint, 0.6),
                }
            }
        }
        self.list.scrollbar(g, Metric::ALL.len(), top, bottom);
        g.hint(&format!("← {} →", read[chosen].title()));
    }
}

/// A measure's name as a row carries it: Sans, or in Bezel's theme the
/// lowercase mono voice, a size down so the value and bars keep their room.
fn name_and_style(g: &Gfx, metric: Metric) -> (String, bezel::TextStyle) {
    if g.theme == crate::look::Theme::Bezel {
        (metric.name().to_lowercase(), g.ty.caption)
    } else {
        (metric.name().to_owned(), g.ty.body)
    }
}

/// The widest value a measure shows, flagged.
#[cfg(test)]
fn widest(metric: Metric) -> String {
    format!("▲ {}", metric.show(match metric {
        Metric::Battery => 12.9,
        Metric::Brownouts | Metric::Overruns => 999.0,
        Metric::Hottest => 109.0,
        Metric::Can => 1.0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Row> {
        crate::logs::Logs::demo().list()
    }

    #[test]
    fn the_demo_event_shows_a_robot_getting_worse() {
        let rows = rows();
        assert!(available(&rows));
        let read = matches(&rows);
        assert_eq!(read.len(), 7, "the practice log is not read yet");
        assert!(read.first().unwrap().name.contains("Q1") && read.last().unwrap().name.contains("Q14"), "oldest first");
        let worse = worsening(&rows);
        for metric in [Metric::Battery, Metric::Brownouts, Metric::Overruns, Metric::Hottest, Metric::Can] {
            assert!(worse.contains(&metric), "{metric:?} in {worse:?}");
        }
        let (flagged, line) = headline(&rows);
        assert!(flagged);
        assert_eq!(line, "battery sagging lower · 4 more");
    }

    #[test]
    fn a_steady_robot_is_not_flagged() {
        let steady = |battery: f64, temp: f64| Digest {
            seconds: 150.0,
            enabled_seconds: Some(135.0),
            battery_low: Some(battery),
            worst_loop_ms: Some(8.0),
            highlights: Vec::new(),
            brownouts: Some(0),
            loop_overruns: Some(1),
            hottest: Some(("Elevator".into(), temp)),
            can_peak: Some(0.4),
        };
        let rows: Vec<Row> = [(11.8, 55.0), (11.6, 58.0), (11.9, 54.0), (11.7, 57.0)]
            .iter()
            .enumerate()
            .map(|(i, (b, t))| Row { name: format!("FRC_2026111{}_100000.wpilog", 9 - i), bytes: 1, digest: Some(steady(*b, *t)) })
            .collect();
        assert!(worsening(&rows).is_empty());
        assert_eq!(headline(&rows), (false, "nothing getting worse".to_owned()));
        // A battery that suddenly sags much lower is.
        let mut sagging = rows.clone();
        sagging[0].digest.as_mut().unwrap().battery_low = Some(10.9);
        assert_eq!(worsening(&sagging), [Metric::Battery]);
    }

    #[test]
    fn every_measure_and_its_words_fit_the_panel() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for theme in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            g.theme = theme;
            for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
                g.set_text_size(size);
                let ty = g.ty;
                // A row's name beside its widest value, flagged, before the bars.
                let row_w = W - 2.0 * PAD - 5.0;
                for m in Metric::ALL {
                    let (name, style) = name_and_style(&g, m);
                    let room = row_w - 8.0 - BARS_W - 8.0 - g.measure(&widest(m), ty.label) - 6.0 - 10.0;
                    assert!(g.measure(&name, style) <= room, "{theme:?} {size:?}: {name} in {room}");
                    // And Logs' line for it, with others worse too.
                    let line = format!("{} · 4 more", m.worse_words());
                    assert!(g.measure(&line, ty.caption) <= row_w - 31.0, "{line}");
                }
                // The match line under the bars.
                let title = Row { name: "FRC_20261116_101500_NYRO_QF12.wpilog".into(), bytes: 0, digest: None }.title();
                assert!(g.measure(&format!("← {title} →"), ty.caption) <= W - 2.0 * PAD, "{title}");
            }
        }
    }
}
