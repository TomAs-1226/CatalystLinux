//! The scouting form: a season's game as data, not code. The form ships for
//! 2026 (REBUILT) in `rebuilt-2026.json`; a team can put another in
//! `/data/scout/form.json` for a new season without a new shell.
//!
//! A form has sections (the match's timed phases, a whole-match one whose
//! keys work throughout, and one for after the match) and fields in them:
//! counts, toggles, choices, ratings and text. Each field that is tapped
//! during the match has a key; a key adds, the key with Shift (or the
//! field's `minus` key) takes away. Totals add up fields' points by section.

use std::collections::BTreeMap;

use serde_json::Value;

/// The form shipped for this season.
pub const SHIPPED: &str = include_str!("rebuilt-2026.json");

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// Taps counted, never below nothing.
    Count,
    /// On or off.
    Toggle,
    /// One of a few options, each worth its points; the first is "none".
    Choice(Vec<(String, f32)>),
    /// 1 to `max`; 0 is not rated yet.
    Rating(u32),
    /// Words, up to so many characters.
    Text(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub id: String,
    pub name: String,
    /// How long the phase runs on the match clock; 0 for one that is not timed.
    pub seconds: f32,
    /// A section whose keys work here too (end game goes on counting TELEOP's).
    pub inherits: Option<usize>,
    /// Its keys work through the whole match (fouls).
    pub always: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub section: usize,
    pub id: String,
    pub label: String,
    pub kind: Kind,
    pub key: Option<char>,
    pub minus: Option<char>,
    /// Points for each count, or for a toggle that is on.
    pub points: f32,
    /// Fewer is better (fouls).
    pub lower_better: bool,
}

impl Field {
    /// Where its value is kept in a record: `auto.fuel`.
    pub fn column(&self, form: &Form) -> String {
        format!("{}.{}", form.sections[self.section].id, self.id)
    }

    /// The points a value of it scores.
    pub fn points_of(&self, value: f32) -> f32 {
        match &self.kind {
            Kind::Count | Kind::Toggle => self.points * value,
            Kind::Choice(options) => options.get(value.max(0.0) as usize).map_or(0.0, |o| o.1),
            Kind::Rating(_) | Kind::Text(_) => 0.0,
        }
    }

    /// The value one step along (`by` is +1 or −1), kept in range. A choice
    /// goes round; a toggle flips whichever way.
    pub fn stepped(&self, value: f32, by: i32) -> f32 {
        match &self.kind {
            Kind::Toggle => 1.0 - value.clamp(0.0, 1.0),
            Kind::Choice(options) => {
                let n = options.len().max(1) as i32;
                ((value as i32 + by).rem_euclid(n)) as f32
            }
            Kind::Count => (value + by as f32).clamp(0.0, 999.0),
            Kind::Rating(max) => (value + by as f32).clamp(0.0, *max as f32),
            Kind::Text(_) => value,
        }
    }

    /// How a value of it reads: "12", "yes", "L2", "4 of 5".
    pub fn shown(&self, value: f32) -> String {
        match &self.kind {
            Kind::Count => format!("{}", value as i64),
            Kind::Toggle => (if value > 0.5 { "yes" } else { "no" }).into(),
            Kind::Choice(options) => options.get(value.max(0.0) as usize).map_or_else(String::new, |o| o.0.clone()),
            Kind::Rating(_) if value < 0.5 => "–".into(),
            Kind::Rating(max) => format!("{} of {max}", value as i64),
            Kind::Text(_) => String::new(),
        }
    }
}

/// A sum of points over sections: the match's points, AUTO's, fouls given.
#[derive(Clone, Debug, PartialEq)]
pub struct Total {
    pub id: String,
    pub label: String,
    pub of: Vec<usize>,
    pub lower_better: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub id: String,
    pub game: String,
    pub season: u32,
    pub sections: Vec<Section>,
    pub fields: Vec<Field>,
    pub totals: Vec<Total>,
}

/// A number from JSON as f32.
fn num(v: &Value) -> Option<f32> {
    v.as_f64().map(|n| n as f32)
}

fn key_of(v: &Value) -> Result<Option<char>, String> {
    match v.as_str() {
        None => Ok(None),
        Some(s) => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_lowercase() || ";',./[]-=`".contains(c) => Ok(Some(c)),
                _ => Err(format!("{s:?} is not a key: one lowercase letter or mark")),
            }
        }
    }
}

impl Form {
    /// The form shipped with the shell.
    pub fn shipped() -> Form {
        Form::parse(SHIPPED).expect("the shipped form reads")
    }

    /// A form from its JSON, checked: every section a field names exists,
    /// ids are not repeated, and no key means two things where it works.
    pub fn parse(text: &str) -> Result<Form, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        let id = v["id"].as_str().filter(|s| !s.is_empty()).ok_or("no id")?.to_owned();
        let raw_sections = v["sections"].as_array().filter(|a| !a.is_empty()).ok_or("no sections")?;
        let mut sections = Vec::new();
        for s in raw_sections {
            let sid = s["id"].as_str().ok_or("a section with no id")?.to_owned();
            if sections.iter().any(|x: &Section| x.id == sid) {
                return Err(format!("section {sid} twice"));
            }
            sections.push(Section {
                name: s["name"].as_str().unwrap_or(&sid).to_owned(),
                id: sid,
                seconds: num(&s["seconds"]).unwrap_or(0.0).max(0.0),
                inherits: None,
                always: s["always"].as_bool().unwrap_or(false),
            });
        }
        let ids: Vec<String> = sections.iter().map(|s| s.id.clone()).collect();
        let section_of = |name: &str| ids.iter().position(|s| s == name);
        for (s, raw) in sections.iter_mut().zip(raw_sections) {
            s.inherits = match raw["inherits"].as_str() {
                Some(name) => Some(section_of(name).ok_or(format!("no section {name} to inherit"))?),
                None => None,
            };
        }
        let mut fields: Vec<Field> = Vec::new();
        for f in v["fields"].as_array().ok_or("no fields")? {
            let fid = f["id"].as_str().ok_or("a field with no id")?.to_owned();
            let sname = f["section"].as_str().ok_or(format!("{fid} has no section"))?;
            let section = section_of(sname).ok_or(format!("{fid}: no section {sname}"))?;
            if fields.iter().any(|x| x.section == section && x.id == fid) {
                return Err(format!("{sname}.{fid} twice"));
            }
            let kind = match f["kind"].as_str().unwrap_or("count") {
                "count" => Kind::Count,
                "toggle" => Kind::Toggle,
                "rating" => Kind::Rating(f["max"].as_u64().unwrap_or(5).clamp(1, 9) as u32),
                "text" => Kind::Text(f["max"].as_u64().unwrap_or(160).clamp(1, 400) as usize),
                "choice" => {
                    let options: Vec<(String, f32)> = f["options"]
                        .as_array()
                        .ok_or(format!("{fid}: a choice needs options"))?
                        .iter()
                        .filter_map(|o| match o {
                            Value::String(s) => Some((s.clone(), 0.0)),
                            Value::Array(a) => Some((a.first()?.as_str()?.to_owned(), a.get(1).and_then(num).unwrap_or(0.0))),
                            _ => None,
                        })
                        .collect();
                    if options.len() < 2 {
                        return Err(format!("{fid}: a choice needs two options or more"));
                    }
                    Kind::Choice(options)
                }
                other => return Err(format!("{fid}: no kind {other}")),
            };
            fields.push(Field {
                section,
                label: f["label"].as_str().unwrap_or(&fid).to_owned(),
                id: fid,
                kind,
                key: key_of(&f["key"])?,
                minus: key_of(&f["minus"])?,
                points: num(&f["points"]).unwrap_or(0.0),
                lower_better: f["better"].as_str() == Some("lower"),
            });
        }
        let mut totals = Vec::new();
        for t in v["totals"].as_array().into_iter().flatten() {
            let tid = t["id"].as_str().ok_or("a total with no id")?.to_owned();
            let of = t["of"].as_array().ok_or(format!("{tid}: no sections"))?.iter().map(|s| s.as_str().and_then(section_of).ok_or(format!("{tid}: no such section"))).collect::<Result<Vec<_>, _>>()?;
            totals.push(Total { label: t["label"].as_str().unwrap_or(&tid).to_owned(), id: tid, of, lower_better: t["better"].as_str() == Some("lower") });
        }
        let form = Form { id, game: v["game"].as_str().unwrap_or("").to_owned(), season: v["season"].as_u64().unwrap_or(0) as u32, sections, fields, totals };
        form.check_keys()?;
        Ok(form)
    }

    /// No key may mean two things in a section where it works.
    fn check_keys(&self) -> Result<(), String> {
        for s in 0..self.sections.len() {
            let mut seen: BTreeMap<char, String> = BTreeMap::new();
            for f in self.reachable(s).map(|i| &self.fields[i]) {
                for k in [f.key, f.minus].into_iter().flatten() {
                    if let Some(other) = seen.insert(k, f.label.clone()) {
                        return Err(format!("key {k} is both {other} and {} in {}", f.label, self.sections[s].name));
                    }
                }
            }
        }
        Ok(())
    }

    /// The fields whose keys work in section `s`: its own, those of the one
    /// it inherits, and the whole-match ones.
    pub fn reachable(&self, s: usize) -> impl Iterator<Item = usize> + '_ {
        let inherited = self.sections.get(s).and_then(|x| x.inherits);
        (0..self.fields.len()).filter(move |&i| {
            let fs = self.fields[i].section;
            fs == s || Some(fs) == inherited || (self.sections[fs].always && fs != s)
        })
    }

    /// What key `c` does in section `s`: the field, and +1 or −1. Shift with
    /// a field's key takes one away, as its `minus` key does.
    pub fn key_in(&self, s: usize, c: char) -> Option<(usize, i32)> {
        let lower = c.to_ascii_lowercase();
        self.reachable(s).find_map(|i| {
            let f = &self.fields[i];
            if f.key == Some(c) {
                Some((i, 1))
            } else if f.minus == Some(c) || (c.is_ascii_uppercase() && f.key == Some(lower)) {
                Some((i, -1))
            } else {
                None
            }
        })
    }

    /// The sections the match clock runs through, in order, with their seconds.
    pub fn timed(&self) -> Vec<usize> {
        (0..self.sections.len()).filter(|&s| self.sections[s].seconds > 0.0).collect()
    }

    /// The whole match, seconds.
    pub fn match_seconds(&self) -> f32 {
        self.sections.iter().map(|s| s.seconds).sum()
    }

    /// The section the clock is in, `elapsed` seconds into the match; past
    /// the end, the first untimed section that is not whole-match (after).
    pub fn section_at(&self, elapsed: f32) -> usize {
        let mut until = 0.0;
        for s in self.timed() {
            until += self.sections[s].seconds;
            if elapsed < until {
                return s;
            }
        }
        self.after().unwrap_or(0)
    }

    /// The section for after the match, if the form has one.
    pub fn after(&self) -> Option<usize> {
        (0..self.sections.len()).find(|&s| self.sections[s].seconds == 0.0 && !self.sections[s].always)
    }

    /// The sections a scout turns through: the timed ones, then after.
    pub fn pages(&self) -> Vec<usize> {
        let mut pages = self.timed();
        pages.extend(self.after());
        pages
    }

    /// A short tag for the form's columns, so codes made with another form
    /// are told apart.
    pub fn tag(&self) -> String {
        let columns: Vec<String> = self.fields.iter().map(|f| f.column(self)).collect();
        format!("{}.{}", self.id, &super::share::hash(columns.join(",").as_bytes())[..4])
    }

    /// The points of a record's values over the sections a total covers.
    pub fn total(&self, total: &Total, values: &BTreeMap<String, f32>) -> f32 {
        self.fields.iter().filter(|f| total.of.contains(&f.section)).map(|f| f.points_of(values.get(&f.column(self)).copied().unwrap_or(0.0))).sum()
    }
}

// ---------------------------------------------------------------- metrics ---

/// Something worth comparing teams by: a total, or a field.
#[derive(Clone, Debug, PartialEq)]
pub enum Metric {
    Total(usize),
    Field(usize),
}

impl Form {
    /// Every metric: the totals first, then each field that holds a number.
    pub fn metrics(&self) -> Vec<Metric> {
        let mut out: Vec<Metric> = (0..self.totals.len()).map(Metric::Total).collect();
        out.extend((0..self.fields.len()).filter(|&i| !matches!(self.fields[i].kind, Kind::Text(_))).map(Metric::Field));
        out
    }

    /// A metric's name in a list: "points", "auto fuel in".
    pub fn metric_label(&self, m: &Metric) -> String {
        match m {
            Metric::Total(i) => self.totals[*i].label.clone(),
            Metric::Field(i) => {
                let f = &self.fields[*i];
                let s = &self.sections[f.section];
                if s.always || s.seconds == 0.0 {
                    f.label.clone()
                } else {
                    format!("{} {}", s.name, f.label)
                }
            }
        }
    }

    pub fn lower_better(&self, m: &Metric) -> bool {
        match m {
            Metric::Total(i) => self.totals[*i].lower_better,
            Metric::Field(i) => self.fields[*i].lower_better,
        }
    }

    /// A metric's value in one record: points for a total or a choice, the
    /// count, 1 or 0 for a toggle, the rating (none when not rated).
    pub fn metric_value(&self, m: &Metric, values: &BTreeMap<String, f32>) -> Option<f32> {
        match m {
            Metric::Total(i) => Some(self.total(&self.totals[*i], values)),
            Metric::Field(i) => {
                let f = &self.fields[*i];
                let v = values.get(&f.column(self)).copied().unwrap_or(0.0);
                match f.kind {
                    Kind::Choice(_) => Some(f.points_of(v)),
                    Kind::Rating(_) if v < 0.5 => None,
                    _ => Some(v),
                }
            }
        }
    }

    /// An average of the metric as it reads: "42.5", "60%" for a toggle.
    pub fn metric_text(&self, m: &Metric, avg: Option<f32>) -> String {
        let Some(v) = avg else { return "–".into() };
        match m {
            Metric::Field(i) if self.fields[*i].kind == Kind::Toggle => format!("{:.0}%", v * 100.0),
            _ if v >= 100.0 || (v - v.round()).abs() < 0.05 => format!("{v:.0}"),
            _ => format!("{v:.1}"),
        }
    }
}
