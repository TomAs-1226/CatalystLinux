//! The game itself, with nothing drawn: the fleet, one side's waters (its
//! ships and the shots fired into them), and what the player firing into
//! them can see.

use catalyst_kit::Rng;

/// The board is N by N squares.
pub const N: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Carrier,
    Battleship,
    Cruiser,
    Submarine,
    Destroyer,
}

/// Each side's fleet, in the order it is placed.
pub const FLEET: [Kind; 5] = [Kind::Carrier, Kind::Battleship, Kind::Cruiser, Kind::Submarine, Kind::Destroyer];

impl Kind {
    pub fn len(self) -> usize {
        match self {
            Kind::Carrier => 5,
            Kind::Battleship => 4,
            Kind::Cruiser | Kind::Submarine => 3,
            Kind::Destroyer => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Carrier => "carrier",
            Kind::Battleship => "battleship",
            Kind::Cruiser => "cruiser",
            Kind::Submarine => "submarine",
            Kind::Destroyer => "destroyer",
        }
    }

    /// Its place in [`FLEET`].
    pub fn index(self) -> usize {
        self as usize
    }
}

/// A ship lying from its top-left square, across or down.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ship {
    pub kind: Kind,
    pub x: usize,
    pub y: usize,
    pub across: bool,
}

impl Ship {
    pub fn new(kind: Kind, x: usize, y: usize, across: bool) -> Ship {
        Ship { kind, x, y, across }
    }

    /// The columns and rows it spans.
    pub fn size(self) -> (usize, usize) {
        if self.across {
            (self.kind.len(), 1)
        } else {
            (1, self.kind.len())
        }
    }

    pub fn cells(self) -> impl Iterator<Item = (usize, usize)> {
        let (dx, dy) = if self.across { (1, 0) } else { (0, 1) };
        (0..self.kind.len()).map(move |i| (self.x + dx * i, self.y + dy * i))
    }

    pub fn on_board(self) -> bool {
        let (w, h) = self.size();
        self.x + w <= N && self.y + h <= N
    }

    pub fn covers(self, x: usize, y: usize) -> bool {
        let (w, h) = self.size();
        (self.x..self.x + w).contains(&x) && (self.y..self.y + h).contains(&y)
    }

    /// Whether the two share a square. Ships may touch; they may not overlap.
    pub fn overlaps(self, other: Ship) -> bool {
        self.cells().any(|(x, y)| other.covers(x, y))
    }

    /// The same ship moved so that it lies wholly on the board.
    pub fn kept_on_board(self) -> Ship {
        let (w, h) = self.size();
        Ship { x: self.x.min(N - w), y: self.y.min(N - h), ..self }
    }
}

/// A square as the player firing into it sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Unknown,
    Miss,
    /// Part of a ship that is still afloat.
    Hit,
    /// Part of a ship that has gone down: the whole ship is known.
    Sunk,
}

/// What a shot found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shot {
    Miss,
    Hit(Kind),
    Sunk(Kind),
}

/// One side's waters: its ships, and the squares fired at.
#[derive(Clone, Debug)]
pub struct Board {
    ships: Vec<Ship>,
    fired: [bool; N * N],
    shots: u32,
    hits: u32,
}

impl Default for Board {
    fn default() -> Self {
        Board::new()
    }
}

impl Board {
    pub fn new() -> Board {
        Board { ships: Vec::with_capacity(FLEET.len()), fired: [false; N * N], shots: 0, hits: 0 }
    }

    /// A whole fleet, placed at random.
    pub fn random(rng: &mut Rng) -> Board {
        let mut board = Board::new();
        board.place_rest_at_random(rng);
        board
    }

    /// The ships placed so far, in [`FLEET`] order.
    pub fn ships(&self) -> &[Ship] {
        &self.ships
    }

    /// The next ship of the fleet to place, if any is left.
    pub fn next_kind(&self) -> Option<Kind> {
        FLEET.get(self.ships.len()).copied()
    }

    pub fn complete(&self) -> bool {
        self.ships.len() == FLEET.len()
    }

    /// The placed ship that `ship` would overlap, if any.
    pub fn clash(&self, ship: Ship) -> Option<Ship> {
        self.ships.iter().copied().find(|s| s.overlaps(ship))
    }

    /// Whether `ship` may go down next: the next of the fleet, on the
    /// board, over no other ship.
    pub fn can_place(&self, ship: Ship) -> bool {
        Some(ship.kind) == self.next_kind() && ship.on_board() && self.clash(ship).is_none()
    }

    pub fn place(&mut self, ship: Ship) -> bool {
        let ok = self.can_place(ship);
        if ok {
            self.ships.push(ship);
        }
        ok
    }

    /// Take back the last ship placed.
    pub fn unplace(&mut self) -> Option<Ship> {
        self.ships.pop()
    }

    /// Every place the next ship could legally go.
    fn places(&self, kind: Kind) -> impl Iterator<Item = Ship> + '_ {
        [true, false].into_iter().flat_map(move |across| (0..N * N).map(move |i| Ship::new(kind, i % N, i / N, across))).filter(move |s| s.on_board() && self.clash(*s).is_none())
    }

    /// Place what is left of the fleet at random, each ship in one of the
    /// places still open to it, chosen evenly. The ships already placed stay
    /// unless they leave no room at all (they never do on a 10 × 10 board).
    pub fn place_rest_at_random(&mut self, rng: &mut Rng) {
        let mut kept = self.ships.len();
        let mut attempts = 0;
        loop {
            while let Some(kind) = self.next_kind() {
                let open = self.places(kind).count();
                if open == 0 {
                    break;
                }
                let pick = rng.below(open);
                let Some(ship) = self.places(kind).nth(pick) else { break };
                self.ships.push(ship);
            }
            if self.complete() {
                return;
            }
            attempts += 1;
            if attempts == 64 {
                kept = 0;
            }
            self.ships.truncate(kept);
        }
    }

    /// The nearest place to `from` where the next ship could go: the same
    /// way round if it can be, below before above.
    pub fn nearest_open(&self, from: Ship) -> Option<Ship> {
        let kind = self.next_kind()?;
        let score = |s: Ship| (s.across != from.across, s.x.abs_diff(from.x) + s.y.abs_diff(from.y), s.y < from.y, s.x);
        self.places(kind).min_by_key(|s| score(*s))
    }

    pub fn ship_at(&self, x: usize, y: usize) -> Option<Ship> {
        self.ships.iter().copied().find(|s| s.covers(x, y))
    }

    pub fn fired(&self, x: usize, y: usize) -> bool {
        self.fired[y * N + x]
    }

    pub fn is_sunk(&self, ship: Ship) -> bool {
        ship.cells().all(|(x, y)| self.fired(x, y))
    }

    /// Fire at a square. `None` if it was fired at already (or is off the board).
    pub fn fire(&mut self, x: usize, y: usize) -> Option<Shot> {
        if x >= N || y >= N || self.fired(x, y) {
            return None;
        }
        self.fired[y * N + x] = true;
        self.shots += 1;
        Some(match self.ship_at(x, y) {
            None => Shot::Miss,
            Some(ship) => {
                self.hits += 1;
                if self.is_sunk(ship) {
                    Shot::Sunk(ship.kind)
                } else {
                    Shot::Hit(ship.kind)
                }
            }
        })
    }

    /// A square as the player firing into these waters sees it.
    pub fn mark(&self, x: usize, y: usize) -> Mark {
        if !self.fired(x, y) {
            return Mark::Unknown;
        }
        match self.ship_at(x, y) {
            None => Mark::Miss,
            Some(ship) if self.is_sunk(ship) => Mark::Sunk,
            Some(_) => Mark::Hit,
        }
    }

    pub fn all_sunk(&self) -> bool {
        !self.ships.is_empty() && self.ships.iter().all(|s| self.is_sunk(*s))
    }

    /// Shots fired into these waters, and how many found a ship.
    pub fn shots(&self) -> u32 {
        self.shots
    }

    pub fn hits(&self) -> u32 {
        self.hits
    }

    /// Ships of this fleet gone down.
    pub fn sunk_count(&self) -> usize {
        self.ships.iter().filter(|s| self.is_sunk(**s)).count()
    }

    /// All that the other player may know of these waters.
    pub fn sight(&self) -> Sight {
        let mut marks = [Mark::Unknown; N * N];
        for (i, mark) in marks.iter_mut().enumerate() {
            *mark = self.mark(i % N, i / N);
        }
        let mut afloat = [false; FLEET.len()];
        for ship in &self.ships {
            afloat[ship.kind.index()] = !self.is_sunk(*ship);
        }
        Sight { marks, afloat }
    }
}

/// What the player firing into a side's waters knows of them: each square's
/// mark, and which of the fleet are still afloat (a sinking is announced).
/// The computer player sees nothing else.
#[derive(Clone, Debug)]
pub struct Sight {
    pub marks: [Mark; N * N],
    /// By [`FLEET`] index.
    pub afloat: [bool; FLEET.len()],
}

impl Sight {
    pub fn mark(&self, x: usize, y: usize) -> Mark {
        self.marks[y * N + x]
    }

    /// The lengths of the ships still afloat.
    pub fn afloat_lens(&self) -> impl Iterator<Item = usize> + '_ {
        FLEET.iter().zip(self.afloat).filter(|(_, afloat)| *afloat).map(|(kind, _)| kind.len())
    }

    /// The shortest ship still afloat (0 when none is).
    pub fn shortest_afloat(&self) -> usize {
        self.afloat_lens().min().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legal(board: &Board) -> bool {
        let ships = board.ships();
        ships.len() == FLEET.len()
            && ships.iter().zip(FLEET).all(|(s, kind)| s.kind == kind && s.on_board())
            && ships.iter().enumerate().all(|(i, a)| ships[i + 1..].iter().all(|b| !a.overlaps(*b)))
    }

    #[test]
    fn zz_print_gallery_fleet() {
        let board = Board::random(&mut Rng::new(5805));
        for s in board.ships() {
            eprintln!("{:?}", s);
        }
    }

    #[test]
    fn random_placement_is_always_legal() {
        for seed in 0..2000 {
            let board = Board::random(&mut Rng::new(seed));
            assert!(legal(&board), "seed {seed}: {:?}", board.ships());
            let covered = (0..N * N).filter(|i| board.ship_at(i % N, i / N).is_some()).count();
            assert_eq!(covered, 17, "seed {seed}: 17 squares of ship, none shared");
        }
    }

    #[test]
    fn the_rest_go_down_at_random_around_the_ships_placed_by_hand() {
        for seed in 0..300 {
            let mut rng = Rng::new(seed);
            let mut board = Board::new();
            assert!(board.place(Ship::new(Kind::Carrier, 2, 4, true)));
            assert!(board.place(Ship::new(Kind::Battleship, 6, 5, false)));
            board.place_rest_at_random(&mut rng);
            assert!(legal(&board), "seed {seed}");
            assert_eq!(board.ships()[0], Ship::new(Kind::Carrier, 2, 4, true), "the hand-placed ships stay");
            assert_eq!(board.ships()[1], Ship::new(Kind::Battleship, 6, 5, false));
        }
    }

    #[test]
    fn ships_may_touch_but_not_overlap() {
        let mut board = Board::new();
        assert!(board.place(Ship::new(Kind::Carrier, 0, 0, true)));
        assert!(!board.place(Ship::new(Kind::Battleship, 4, 0, false)), "overlaps the carrier's end");
        assert!(!board.place(Ship::new(Kind::Battleship, 7, 0, true)), "off the board");
        assert!(!board.place(Ship::new(Kind::Cruiser, 0, 2, true)), "not the next of the fleet");
        assert!(board.place(Ship::new(Kind::Battleship, 0, 1, true)), "touching is fine");
        assert_eq!(board.unplace().map(|s| s.kind), Some(Kind::Battleship));
        assert_eq!(board.next_kind(), Some(Kind::Battleship));
        let next = board.nearest_open(Ship::new(Kind::Carrier, 0, 0, true)).unwrap();
        assert_eq!((next.x, next.y, next.across), (0, 1, true), "just below");
    }

    #[test]
    fn shots_hit_sink_and_are_never_counted_twice() {
        let mut board = Board::new();
        board.place(Ship::new(Kind::Carrier, 0, 0, true));
        assert_eq!(board.fire(9, 9), Some(Shot::Miss));
        assert_eq!(board.fire(9, 9), None);
        for x in 0..4 {
            assert_eq!(board.fire(x, 0), Some(Shot::Hit(Kind::Carrier)));
            assert_eq!(board.mark(x, 0), Mark::Hit);
        }
        assert_eq!(board.fire(4, 0), Some(Shot::Sunk(Kind::Carrier)));
        assert_eq!(board.mark(0, 0), Mark::Sunk);
        assert_eq!(board.mark(9, 9), Mark::Miss);
        assert!(board.all_sunk());
        assert_eq!((board.shots(), board.hits()), (6, 5));
        let sight = board.sight();
        assert_eq!(sight.afloat_lens().count(), 0);
    }
}
