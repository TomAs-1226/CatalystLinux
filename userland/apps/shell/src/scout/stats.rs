//! What the records say about each team: every metric match by match, its
//! average and trend; teams sorted by any metric; and the picklist, ordered
//! by hand, with notes and do-not-pick flags.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::form::Form;
use super::store::Record;

/// A team's records, metric by metric, in match order.
#[derive(Clone, Debug, PartialEq)]
pub struct Team {
    pub team: u32,
    pub matches: Vec<String>,
    /// For each of the form's metrics, its value in each match (none for a
    /// rating not given).
    pub series: Vec<Vec<Option<f32>>>,
}

impl Team {
    /// A metric's average over the matches that have it.
    pub fn avg(&self, metric: usize) -> Option<f32> {
        let got: Vec<f32> = self.series.get(metric)?.iter().flatten().copied().collect();
        (!got.is_empty()).then(|| got.iter().sum::<f32>() / got.len() as f32)
    }

    /// The metric's values, matches without one left out.
    pub fn values(&self, metric: usize) -> Vec<f32> {
        self.series.get(metric).map(|s| s.iter().flatten().copied().collect()).unwrap_or_default()
    }

    /// The change from the first half of its matches to the second, for an
    /// arrow: positive when it has risen.
    pub fn trend(&self, metric: usize) -> Option<f32> {
        let v = self.values(metric);
        if v.len() < 3 {
            return None;
        }
        let half = v.len() / 2;
        let mean = |s: &[f32]| s.iter().sum::<f32>() / s.len() as f32;
        Some(mean(&v[v.len() - half..]) - mean(&v[..half]))
    }
}

/// Every team with records at the event, by team number.
pub fn teams(form: &Form, records: &[&Record]) -> Vec<Team> {
    let metrics = form.metrics();
    let mut numbers: Vec<u32> = records.iter().map(|r| r.team).collect();
    numbers.sort_unstable();
    numbers.dedup();
    numbers
        .into_iter()
        .map(|team| {
            let mut mine: Vec<&&Record> = records.iter().filter(|r| r.team == team).collect();
            mine.sort_by_key(|r| super::store::match_order(&r.match_key));
            Team {
                team,
                matches: mine.iter().map(|r| r.label.clone()).collect(),
                series: metrics.iter().map(|m| mine.iter().map(|r| form.metric_value(m, &r.values)).collect()).collect(),
            }
        })
        .collect()
}

/// Teams in order of a metric, best first (fewest first where fewer is
/// better), teams without it last, ties by team number.
pub fn sort_by(form: &Form, teams: &mut [Team], metric: usize) {
    let lower = form.metrics().get(metric).is_some_and(|m| form.lower_better(m));
    teams.sort_by(|a, b| {
        let (x, y) = (a.avg(metric), b.avg(metric));
        match (x, y) {
            (Some(x), Some(y)) => {
                let o = if lower { x.total_cmp(&y) } else { y.total_cmp(&x) };
                o.then(a.team.cmp(&b.team))
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.team.cmp(&b.team),
        }
    });
}

/// The highest a metric reaches at the event: one scale for every team's line.
pub fn scale(teams: &[Team], metric: usize) -> f32 {
    teams.iter().flat_map(|t| t.values(metric)).fold(0.0, f32::max)
}

/// Index of a metric in the form's list, by its id.
#[cfg(test)]
pub fn metric_index(form: &Form, id: &str) -> Option<usize> {
    use super::form::Metric;
    form.metrics().iter().position(|m| match m {
        Metric::Total(i) => form.totals[*i].id == id,
        Metric::Field(i) => form.fields[*i].column(form) == id,
    })
}

// ---------------------------------------------------------------- picklist ---

#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub team: u32,
    pub note: String,
    pub dnp: bool,
}

/// The picklist for an event: picks first, in the order the strategy lead
/// put them; teams not to pick after them.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Picklist {
    pub event: String,
    pub picks: Vec<Pick>,
}

impl Picklist {
    fn file(root: &Path, event: &str) -> PathBuf {
        let name: String = event.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
        root.join(format!("picklist-{name}.json"))
    }

    pub fn load(root: &Path, event: &str) -> Picklist {
        let picks = std::fs::read_to_string(Self::file(root, event))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["picks"].as_array().cloned())
            .map(|a| a.iter().filter_map(|p| Some(Pick { team: p["team"].as_u64()? as u32, note: p["note"].as_str().unwrap_or("").to_owned(), dnp: p["dnp"].as_bool().unwrap_or(false) })).collect())
            .unwrap_or_default();
        Picklist { event: event.to_owned(), picks }
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        let v = json!({"event": self.event, "picks": self.picks.iter().map(|p| json!({"team": p.team, "note": p.note, "dnp": p.dnp})).collect::<Vec<_>>()});
        crate::sys::write_atomic(&Self::file(root, &self.event), v.to_string().as_bytes())
    }

    /// Teams not on it yet join it, in the order given (best first), above
    /// the teams not to pick. True when any joined.
    pub fn add_missing(&mut self, ranked: &[u32]) -> bool {
        let mut joined = false;
        for team in ranked {
            if self.picks.iter().all(|p| p.team != *team) {
                let at = self.first_dnp();
                self.picks.insert(at, Pick { team: *team, note: String::new(), dnp: false });
                joined = true;
            }
        }
        joined
    }

    /// Where the teams not to pick begin.
    pub fn first_dnp(&self) -> usize {
        self.picks.iter().position(|p| p.dnp).unwrap_or(self.picks.len())
    }

    /// Move the team at `at` up (−1) or down (+1) within its group (picks
    /// among picks); where it lands.
    pub fn shift(&mut self, at: usize, by: i32) -> usize {
        let Some(p) = self.picks.get(at) else { return at };
        let (lo, hi) = if p.dnp { (self.first_dnp(), self.picks.len()) } else { (0, self.first_dnp()) };
        let to = (at as i64 + by as i64).clamp(lo as i64, hi as i64 - 1) as usize;
        let p = self.picks.remove(at);
        self.picks.insert(to, p);
        to
    }

    /// Move the team at `at` to the top of its group.
    pub fn to_top(&mut self, at: usize) -> usize {
        let Some(p) = self.picks.get(at) else { return at };
        let to = if p.dnp { self.first_dnp() } else { 0 };
        let p = self.picks.remove(at);
        self.picks.insert(to, p);
        to
    }

    /// Mark a team not to pick (it goes to the top of those), or take the
    /// mark off (it goes to the foot of the picks). Where it lands.
    pub fn toggle_dnp(&mut self, at: usize) -> usize {
        if at >= self.picks.len() {
            return at;
        }
        let mut p = self.picks.remove(at);
        p.dnp = !p.dnp;
        let to = self.first_dnp();
        self.picks.insert(to, p);
        to
    }

    /// A pick's place among the picks: 1 for the first; none for a team not to pick.
    pub fn rank(&self, at: usize) -> Option<usize> {
        self.picks.get(at).filter(|p| !p.dnp).map(|_| at + 1)
    }
}
