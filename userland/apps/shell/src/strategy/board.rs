//! A match's plan: the six robots, and for each phase (auto, teleop,
//! endgame) where they stand, the paths they drive and the notes beside
//! them. Places are field metres, WPILib's, from the blue origin
//! (`field.rs`), whichever side the team is on; the board turns them to
//! the drive team's view as it draws.
//!
//! One compact binary form holds a board on the data partition, in a
//! template, and in the QR codes another handheld scans (`qr.rs`).

use std::path::{Path, PathBuf};

use crate::field::{LENGTH, WIDTH, ZONE};
use crate::schedule::Match;

/// Robots on a board: ours first (the team, then its partners), then the
/// opponents.
pub const ROBOTS: usize = 6;
pub const OURS: usize = 3;

/// The most a board holds of each, so it always fits its codes (`qr.rs`):
/// paths and notes a phase, waypoints a path, letters a note.
pub const MOST_PATHS: usize = 24;
pub const MOST_POINTS: usize = 32;
pub const MOST_NOTES: usize = 12;
pub const MOST_LETTERS: usize = 24;

/// The match's three parts, each a layer of its own on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    Auto,
    Teleop,
    Endgame,
}

impl Phase {
    pub const ALL: [Phase; 3] = [Phase::Auto, Phase::Teleop, Phase::Endgame];

    pub fn name(self) -> &'static str {
        match self {
            Phase::Auto => "auto",
            Phase::Teleop => "teleop",
            Phase::Endgame => "endgame",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn next(self) -> Phase {
        Phase::ALL[(self.index() + 1) % 3]
    }
}

/// A place on the field, metres from the blue origin.
pub type Spot = (f32, f32);

/// A point turned half round: the same place seen from the other alliance.
pub fn flip((x, y): Spot) -> Spot {
    (LENGTH - x, WIDTH - y)
}

/// Kept on the field.
pub fn clamp((x, y): Spot) -> Spot {
    (x.clamp(0.0, LENGTH), y.clamp(0.0, WIDTH))
}

fn distance(a: Spot, b: Spot) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// A robot's drive: from where it is to each waypoint in turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub robot: usize,
    pub points: Vec<Spot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoteKind {
    /// A few words at a place: "0:05 shoot", "wait for 1507".
    Label,
    /// A drop zone: a ring on the field, with words if any.
    Zone,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub kind: NoteKind,
    /// Whose it is, for its colour.
    pub robot: usize,
    pub at: Spot,
    pub text: String,
}

/// One phase of the board.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    /// Where each robot stands as the phase starts, if placed.
    pub markers: [Option<Spot>; ROBOTS],
    pub paths: Vec<Route>,
    pub notes: Vec<Note>,
}

impl Layer {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.notes.is_empty() && self.markers.iter().all(Option::is_none)
    }

    fn turned(&self) -> Layer {
        Layer {
            markers: self.markers.map(|m| m.map(flip)),
            paths: self.paths.iter().map(|p| Route { robot: p.robot, points: p.points.iter().copied().map(flip).collect() }).collect(),
            notes: self.notes.iter().map(|n| Note { at: flip(n.at), ..n.clone() }).collect(),
        }
    }
}

/// What lies under the cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Marker(usize),
    Note(usize),
    Path(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    /// The match's key ("2026nyro_q31"), "practice", or a template's file name.
    pub key: String,
    /// "Q31", or a template's name.
    pub label: String,
    /// The team's alliance is red: its wall is on the right of the field as
    /// WPILib draws it, and on the left as the board shows it.
    pub ours_red: bool,
    /// Team numbers by robot (0: not known).
    pub teams: [u32; ROBOTS],
    pub layers: [Layer; 3],
}

/// Where the board starts its robots: just behind the starting line (the
/// alliance zone's edge, the game manual's 158.6 in), spread across the
/// field, before a hand moves them. My own placing.
fn starting_spots(red: bool) -> [Spot; 3] {
    let x = ZONE - 0.55;
    let spots = [(x, WIDTH * 0.5), (x, WIDTH * 0.2), (x, WIDTH * 0.8)];
    if red {
        spots.map(flip)
    } else {
        spots
    }
}

impl Board {
    /// A board for a match: the team's alliance and the other, with the
    /// robots at their starting line in auto.
    pub fn new(key: &str, label: &str, ours_red: bool, teams: [u32; ROBOTS]) -> Board {
        let mut board = Board { key: key.to_owned(), label: label.to_owned(), ours_red, teams, layers: Default::default() };
        let (ours, theirs) = (starting_spots(ours_red), starting_spots(!ours_red));
        for i in 0..OURS {
            board.layers[0].markers[i] = Some(ours[i]);
            board.layers[0].markers[OURS + i] = Some(theirs[i]);
        }
        board
    }

    /// The board for one of the team's matches, from the schedule.
    pub fn of_match(m: &Match, team: u32) -> Board {
        let ours_red = m.alliance(team) == Some("red");
        let (ours, theirs) = if ours_red { (&m.red, &m.blue) } else { (&m.blue, &m.red) };
        let mut teams = [0; ROBOTS];
        teams[0] = team;
        for (k, t) in ours.iter().filter(|t| **t != team).take(OURS - 1).enumerate() {
            teams[1 + k] = *t;
        }
        for (k, t) in theirs.iter().take(OURS).enumerate() {
            teams[OURS + k] = *t;
        }
        Board::new(&m.key, &m.label, ours_red, teams)
    }

    /// With no match to plan: the team, if known, on the blue side.
    pub fn practice(team: Option<u32>) -> Board {
        let mut teams = [0; ROBOTS];
        teams[0] = team.unwrap_or(0);
        Board::new("practice", "Practice", false, teams)
    }

    pub fn layer(&self, phase: Phase) -> &Layer {
        &self.layers[phase.index()]
    }

    pub fn layer_mut(&mut self, phase: Phase) -> &mut Layer {
        &mut self.layers[phase.index()]
    }

    /// Whether the robot is on the red alliance.
    pub fn red(&self, robot: usize) -> bool {
        (robot < OURS) == self.ours_red
    }

    /// What a robot is called: its team, else its place ("us", "P2", "O1").
    pub fn name(&self, robot: usize) -> String {
        match self.teams[robot] {
            0 if robot == 0 => "us".into(),
            0 if robot < OURS => format!("P{}", robot + 1),
            0 => format!("O{}", robot - OURS + 1),
            team => team.to_string(),
        }
    }

    /// Where a robot is as a phase starts: placed there, else where the
    /// phase before left it.
    pub fn start_of(&self, phase: Phase, robot: usize) -> Option<Spot> {
        let layer = self.layer(phase);
        if let Some(at) = layer.markers[robot] {
            return Some(at);
        }
        let before = phase.index().checked_sub(1).map(|i| Phase::ALL[i])?;
        self.end_of(before, robot)
    }

    /// Where a robot is once its paths in a phase are driven.
    pub fn end_of(&self, phase: Phase, robot: usize) -> Option<Spot> {
        let last = self.layer(phase).paths.iter().rev().find(|p| p.robot == robot).and_then(|p| p.points.last().copied());
        last.or_else(|| self.start_of(phase, robot))
    }

    /// All a robot drives in a phase, from where it starts, as one line.
    pub fn route(&self, phase: Phase, robot: usize) -> Vec<Spot> {
        let mut out: Vec<Spot> = self.start_of(phase, robot).into_iter().collect();
        for path in self.layer(phase).paths.iter().filter(|p| p.robot == robot) {
            for &p in &path.points {
                if out.last().is_none_or(|&last| distance(last, p) > 0.01) {
                    out.push(p);
                }
            }
        }
        out
    }

    /// The thing nearest `at` within `within` metres: a note first (it sits
    /// on top), then a robot, then a path's waypoint.
    pub fn hit(&self, phase: Phase, at: Spot, within: f32) -> Option<Hit> {
        let layer = self.layer(phase);
        let near = |p: Spot| distance(p, at) <= within;
        if let Some(i) = layer.notes.iter().rposition(|n| near(n.at)) {
            return Some(Hit::Note(i));
        }
        if let Some(r) = (0..ROBOTS).find(|&r| self.start_of(phase, r).is_some_and(near)) {
            return Some(Hit::Marker(r));
        }
        layer.paths.iter().rposition(|p| p.points.iter().skip(1).any(|&q| near(q))).map(Hit::Path)
    }

    /// The board as if the team were blue: how a template keeps it, so it
    /// fits a match on either side.
    pub fn as_blue(&self) -> [Layer; 3] {
        if self.ours_red {
            [self.layers[0].turned(), self.layers[1].turned(), self.layers[2].turned()]
        } else {
            self.layers.clone()
        }
    }

    /// A template's layers put on this board, on the team's side.
    pub fn apply(&mut self, template: &Board) {
        let blue = template.as_blue();
        self.layers = if self.ours_red { [blue[0].turned(), blue[1].turned(), blue[2].turned()] } else { blue };
    }

    /// This board kept as a template called `name`.
    pub fn template(&self, name: &str) -> Board {
        Board { key: slug(name), label: name.to_owned(), ours_red: false, teams: [0; ROBOTS], layers: self.as_blue() }
    }

    /// A number that changes with anything drawn in a phase.
    pub fn phase_hash(&self, phase: Phase) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut bytes = Vec::new();
        put_layer(&mut bytes, self.layer(phase));
        bytes.hash(&mut h);
        // A phase inherits where the one before left each robot.
        for r in 0..ROBOTS {
            self.start_of(phase, r).map(|(x, y)| (cm(x), cm(y))).hash(&mut h);
        }
        self.teams.hash(&mut h);
        self.ours_red.hash(&mut h);
        h.finish()
    }

    // ------------------------------------------------------------ bytes ---

    /// The board in its compact form: about 6 bytes a waypoint or robot,
    /// places to the centimetre.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![MAGIC, VERSION, u8::from(self.ours_red)];
        put_str(&mut out, &self.key);
        put_str(&mut out, &self.label);
        for t in self.teams {
            out.extend_from_slice(&(t.min(u16::MAX as u32) as u16).to_le_bytes());
        }
        for layer in &self.layers {
            put_layer(&mut out, layer);
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Board> {
        let mut r = Reader { bytes, at: 0 };
        if r.u8()? != MAGIC || r.u8()? != VERSION {
            return None;
        }
        let ours_red = r.u8()? & 1 == 1;
        let key = r.str()?;
        let label = r.str()?;
        let mut teams = [0; ROBOTS];
        for t in &mut teams {
            *t = u16::from_le_bytes([r.u8()?, r.u8()?]) as u32;
        }
        let mut layers: [Layer; 3] = Default::default();
        for layer in &mut layers {
            *layer = r.layer()?;
        }
        (r.at == bytes.len()).then_some(Board { key, label, ours_red, teams, layers })
    }
}

const MAGIC: u8 = b'S';
const VERSION: u8 = 1;

/// Metres to whole centimetres, as kept.
fn cm(m: f32) -> u16 {
    (m * 100.0).round().clamp(0.0, 4095.0) as u16
}

/// A place in three bytes: twelve bits of centimetres along, twelve across.
fn put_spot(out: &mut Vec<u8>, (x, y): Spot) {
    let (x, y) = (cm(x), cm(y));
    out.extend_from_slice(&[x as u8, (x >> 8) as u8 | ((y & 0xf) << 4) as u8, (y >> 4) as u8]);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let mut end = s.len().min(255);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    out.push(end as u8);
    out.extend_from_slice(&s.as_bytes()[..end]);
}

fn put_layer(out: &mut Vec<u8>, layer: &Layer) {
    let mask = layer.markers.iter().enumerate().fold(0u8, |m, (i, s)| m | (u8::from(s.is_some()) << i));
    out.push(mask);
    for at in layer.markers.iter().flatten() {
        put_spot(out, *at);
    }
    let paths: Vec<&Route> = layer.paths.iter().filter(|p| !p.points.is_empty()).take(MOST_PATHS).collect();
    out.push(paths.len() as u8);
    for p in paths {
        let points = &p.points[..p.points.len().min(MOST_POINTS)];
        out.extend_from_slice(&[p.robot.min(ROBOTS - 1) as u8, points.len() as u8]);
        for at in points {
            put_spot(out, *at);
        }
    }
    let notes = &layer.notes[..layer.notes.len().min(MOST_NOTES)];
    out.push(notes.len() as u8);
    for n in notes {
        let kind = match n.kind {
            NoteKind::Label => 0,
            NoteKind::Zone => 1,
        };
        out.push(kind << 4 | n.robot.min(ROBOTS - 1) as u8);
        put_spot(out, n.at);
        let text: String = n.text.chars().take(MOST_LETTERS).collect();
        put_str(out, &text);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(b)
    }

    fn str(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        let s = self.bytes.get(self.at..self.at + n)?;
        self.at += n;
        String::from_utf8(s.to_vec()).ok()
    }

    fn spot(&mut self) -> Option<Spot> {
        let (a, b, c) = (self.u8()? as u16, self.u8()? as u16, self.u8()? as u16);
        let (x, y) = (a | (b & 0xf) << 8, b >> 4 | c << 4);
        Some(clamp((x as f32 / 100.0, y as f32 / 100.0)))
    }

    fn robot(&mut self, b: u8) -> Option<usize> {
        let r = (b & 0xf) as usize;
        (r < ROBOTS).then_some(r)
    }

    fn layer(&mut self) -> Option<Layer> {
        let mut layer = Layer::default();
        let mask = self.u8()?;
        for (i, m) in layer.markers.iter_mut().enumerate() {
            if mask & (1 << i) != 0 {
                *m = Some(self.spot()?);
            }
        }
        for _ in 0..self.u8()? {
            let b = self.u8()?;
            let robot = self.robot(b)?;
            let n = self.u8()? as usize;
            let points = (0..n).map(|_| self.spot()).collect::<Option<Vec<_>>>()?;
            layer.paths.push(Route { robot, points });
        }
        for _ in 0..self.u8()? {
            let b = self.u8()?;
            let kind = if b >> 4 == 1 { NoteKind::Zone } else { NoteKind::Label };
            let robot = self.robot(b)?;
            let at = self.spot()?;
            layer.notes.push(Note { kind, robot, at, text: self.str()? });
        }
        Some(layer)
    }
}

// ------------------------------------------------------------- storage ---

/// A name made safe for a file: lowercase letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').chars().take(40).collect::<String>();
    if out.is_empty() {
        "plan".into()
    } else {
        out
    }
}

/// Where boards and templates are kept: beside the handheld's settings.
pub fn home() -> PathBuf {
    crate::config_dir().join("strategy")
}

fn board_file(dir: &Path, key: &str) -> PathBuf {
    dir.join("boards").join(format!("{}.board", slug(key)))
}

pub fn load_board(dir: &Path, key: &str) -> Option<Board> {
    std::fs::read(board_file(dir, key)).ok().and_then(|b| Board::decode(&b))
}

pub fn save_board(dir: &Path, board: &Board) -> std::io::Result<()> {
    let file = board_file(dir, &board.key);
    std::fs::create_dir_all(file.parent().unwrap())?;
    crate::sys::write_atomic(&file, &board.encode())
}

/// Whether a plan has been made for a match.
pub fn has_board(dir: &Path, key: &str) -> bool {
    board_file(dir, key).is_file()
}

/// The templates, by name.
pub fn templates(dir: &Path) -> Vec<Board> {
    let mut out: Vec<Board> = std::fs::read_dir(dir.join("templates"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "board"))
        .filter_map(|e| std::fs::read(e.path()).ok().and_then(|b| Board::decode(&b)))
        .collect();
    out.sort_by_key(|t| t.label.to_lowercase());
    out
}

/// Keep a template (one of the same name is replaced).
pub fn save_template(dir: &Path, template: &Board) -> std::io::Result<()> {
    let folder = dir.join("templates");
    std::fs::create_dir_all(&folder)?;
    crate::sys::write_atomic(&folder.join(format!("{}.board", slug(&template.key))), &template.encode())
}

pub fn remove_template(dir: &Path, template: &Board) {
    std::fs::remove_file(dir.join("templates").join(format!("{}.board", slug(&template.key)))).ok();
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn q31() -> Match {
        Match { key: "2026nyro_q31".into(), label: "Q31".into(), red: vec![1507, 5254, 5805], blue: vec![3044, 6328, 2337], ..Default::default() }
    }

    /// A board with something of everything on it.
    pub fn full() -> Board {
        let mut b = Board::of_match(&q31(), 5805);
        let auto = b.layer_mut(Phase::Auto);
        auto.paths.push(Route { robot: 0, points: vec![(12.4, 4.03), (10.9, 6.2), (12.9, 6.9)] });
        auto.paths.push(Route { robot: 4, points: vec![(3.5, 1.6), (6.1, 2.2)] });
        auto.notes.push(Note { kind: NoteKind::Zone, robot: 1, at: (11.2, 1.4), text: "drop".into() });
        auto.notes.push(Note { kind: NoteKind::Label, robot: 0, at: (10.2, 5.4), text: "0:05 shoot".into() });
        b.layer_mut(Phase::Teleop).paths.push(Route { robot: 0, points: vec![(12.9, 6.9), (8.3, 6.5), (12.0, 4.0)] });
        b.layer_mut(Phase::Endgame).markers[2] = Some((13.1, 3.7));
        b
    }

    #[test]
    fn a_match_board_puts_the_team_first_and_each_alliance_at_its_wall() {
        let b = Board::of_match(&q31(), 5805);
        assert_eq!(b.teams, [5805, 1507, 5254, 3044, 6328, 2337]);
        assert!(b.ours_red && b.red(0) && b.red(2) && !b.red(3));
        assert_eq!((b.label.as_str(), b.key.as_str()), ("Q31", "2026nyro_q31"));
        // Red's robots near the red wall (x large), blue's near the blue one.
        let auto = b.layer(Phase::Auto);
        assert!((0..3).all(|r| auto.markers[r].unwrap().0 > LENGTH - ZONE));
        assert!((3..6).all(|r| auto.markers[r].unwrap().0 < ZONE));
        assert_eq!(b.name(1), "1507");
        let practice = Board::practice(None);
        assert_eq!((practice.name(0), practice.name(1), practice.name(4)), ("us".into(), "P2".into(), "O2".into()));
    }

    #[test]
    fn a_phase_starts_where_the_one_before_left_each_robot() {
        let b = full();
        // Robot 0 drove in auto: teleop starts at the end of that path.
        assert_eq!(b.start_of(Phase::Teleop, 0), Some((12.9, 6.9)));
        assert_eq!(b.end_of(Phase::Teleop, 0), Some((12.0, 4.0)));
        // Placed in the endgame: there, whatever came before.
        assert_eq!(b.start_of(Phase::Endgame, 2), Some((13.1, 3.7)));
        // Never moved: still at its starting spot.
        assert_eq!(b.start_of(Phase::Endgame, 5), b.layer(Phase::Auto).markers[5]);
        // Its route runs from the marker through each waypoint, the repeat dropped.
        let route = b.route(Phase::Auto, 0);
        assert_eq!(route.first().copied(), b.layer(Phase::Auto).markers[0]);
        assert_eq!(route.len(), 4);
        assert_eq!(b.route(Phase::Teleop, 0), vec![(12.9, 6.9), (8.3, 6.5), (12.0, 4.0)]);
    }

    #[test]
    fn what_is_under_the_cursor_is_found_notes_first() {
        let b = full();
        assert_eq!(b.hit(Phase::Auto, (10.25, 5.4), 0.5), Some(Hit::Note(1)));
        let marker = b.layer(Phase::Auto).markers[3].unwrap();
        assert_eq!(b.hit(Phase::Auto, marker, 0.5), Some(Hit::Marker(3)));
        assert_eq!(b.hit(Phase::Auto, (6.0, 2.25), 0.5), Some(Hit::Path(1)));
        assert_eq!(b.hit(Phase::Auto, (8.0, 4.0), 0.5), None);
        // A robot placed in no phase until the endgame is found there by its inherited place.
        assert_eq!(b.hit(Phase::Teleop, (12.9, 6.9), 0.3), Some(Hit::Marker(0)));
    }

    #[test]
    fn a_board_comes_back_from_its_bytes_to_the_centimetre() {
        let b = full();
        let bytes = b.encode();
        assert!(bytes.len() < 200, "{} bytes", bytes.len());
        let back = Board::decode(&bytes).expect("decodes");
        assert_eq!(back.encode(), bytes);
        assert_eq!((back.key.as_str(), back.teams, back.ours_red), (b.key.as_str(), b.teams, b.ours_red));
        for (a, z) in b.layers.iter().zip(&back.layers) {
            assert_eq!(a.paths.len(), z.paths.len());
            for (p, q) in a.paths.iter().zip(&z.paths) {
                assert_eq!(p.robot, q.robot);
                assert!(p.points.iter().zip(&q.points).all(|(u, v)| distance(*u, *v) < 0.008));
            }
            assert_eq!(a.notes.iter().map(|n| (&n.text, n.kind, n.robot)).collect::<Vec<_>>(), z.notes.iter().map(|n| (&n.text, n.kind, n.robot)).collect::<Vec<_>>());
        }
        // Anything cut short or not a board is refused.
        assert!(Board::decode(&bytes[..bytes.len() - 1]).is_none());
        assert!(Board::decode(b"hello").is_none());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Board::decode(&longer).is_none());
    }

    #[test]
    fn a_template_keeps_the_plan_on_the_teams_side_whichever_alliance_it_meets() {
        let red = full();
        let t = red.template("our standard auto");
        assert_eq!((t.key.as_str(), t.teams), ("our-standard-auto", [0; ROBOTS]));
        // Kept as if blue: the team's auto path now on the blue half.
        assert!(t.layer(Phase::Auto).paths[0].points[0].0 < LENGTH / 2.0);
        // On a blue match: there it stays. On a red one: turned back.
        let mut blue = Board::of_match(&Match { red: vec![1, 2, 3], blue: vec![5805, 4, 5], ..q31() }, 5805);
        blue.apply(&t);
        assert_eq!(blue.layers, t.layers);
        assert_eq!(blue.teams, [5805, 4, 5, 1, 2, 3], "the match's own teams");
        let mut again = Board::of_match(&q31(), 5805);
        again.apply(&t);
        let (p, q) = (&again.layer(Phase::Auto).paths[0].points, &red.layer(Phase::Auto).paths[0].points);
        assert!(p.iter().zip(q).all(|(u, v)| distance(*u, *v) < 1e-4));
    }

    #[test]
    fn templates_are_kept_by_name_and_boards_by_match() {
        let dir = std::env::temp_dir().join(format!("catalyst-strategy-test-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = full();
        assert!(!has_board(&dir, &b.key));
        save_board(&dir, &b).unwrap();
        assert!(has_board(&dir, "2026nyro_q31"));
        assert_eq!(load_board(&dir, "2026nyro_q31").map(|l| l.encode()), Some(b.encode()));
        save_template(&dir, &b.template("Defense on 1507")).unwrap();
        save_template(&dir, &b.template("our standard auto")).unwrap();
        save_template(&dir, &b.template("defense on 1507")).unwrap();
        let names: Vec<String> = templates(&dir).into_iter().map(|t| t.label).collect();
        assert_eq!(names, ["defense on 1507", "our standard auto"], "the same name replaces");
        remove_template(&dir, &templates(&dir)[0]);
        assert_eq!(templates(&dir).len(), 1);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(slug("  Q31 / défense!! "), "q31-d-fense");
        assert_eq!(slug("!!"), "plan");
    }
}
