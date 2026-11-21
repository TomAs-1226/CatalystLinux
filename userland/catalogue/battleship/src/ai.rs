//! The computer player, as yutacar's plays: by probability density. For
//! every ship still afloat, each place it could lie that covers no miss and
//! no sunk ship adds one to every square it covers; the densest squares are
//! where a ship most likely is.
//!
//! - Target mode, while some hit belongs to a ship still afloat: an
//!   isolated hit proposes its four neighbours, a straight run of hits its
//!   two ends, and it fires at the densest of those, counting the places
//!   that cross the hits far above the rest. Every level targets.
//! - Hunt mode, otherwise: Easy fires at any unknown square; Normal fires
//!   at random 35% of the time and otherwise at the densest square; Hard
//!   fires at the densest square of a checkerboard, (x + y) divisible by the
//!   shortest ship still afloat, as no ship can hide between its squares.
//!
//! Ties are broken at random. It sees only what a player would: a [`Sight`].

use catalyst_kit::Rng;

use crate::game::{Mark, Sight, N};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Easy,
    Normal,
    Hard,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Easy, Level::Normal, Level::Hard];

    pub fn name(self) -> &'static str {
        match self {
            Level::Easy => "Easy",
            Level::Normal => "Normal",
            Level::Hard => "Hard",
        }
    }

    /// Its name in the record file.
    pub fn key(self) -> &'static str {
        match self {
            Level::Easy => "easy",
            Level::Normal => "normal",
            Level::Hard => "hard",
        }
    }

    pub fn from_key(key: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|l| l.key() == key.trim())
    }

    /// How it plays, in a line.
    pub fn about(self) -> &'static str {
        match self {
            Level::Easy => "Fires anywhere until it hits a ship, then closes in on it.",
            Level::Normal => "Hunts where a ship could still be, with a wild shot now and then.",
            Level::Hard => "Hunts a checkerboard by where ships could still lie. Rarely wastes a shot.",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// How often Normal fires at random while hunting.
const NORMAL_WILD: f32 = 0.35;
/// How much more a place that crosses a hit counts, per hit, in target mode.
const ACROSS_HITS: u32 = 64;

/// The square to fire at next, or `None` when every square has been fired at.
pub fn choose(level: Level, sight: &Sight, rng: &mut Rng) -> Option<(usize, usize)> {
    let density = density(sight);
    let at = target(sight, &density, rng).or_else(|| hunt(level, sight, &density, rng))?;
    Some((at % N, at / N))
}

/// For every square, the weight of the places a ship still afloat could lie
/// across it. A place covering a miss or a sunk ship does not count; one
/// crossing hits counts [`ACROSS_HITS`] times over for each.
pub fn density(sight: &Sight) -> [u32; N * N] {
    let mut density = [0u32; N * N];
    for len in sight.afloat_lens() {
        for across in [true, false] {
            let (w, h) = if across { (len, 1) } else { (1, len) };
            for y in 0..=N - h {
                for x in 0..=N - w {
                    let cell = |i: usize| if across { (y * N) + x + i } else { (y + i) * N + x };
                    let mut hits = 0;
                    let mut open = true;
                    for i in 0..len {
                        match sight.marks[cell(i)] {
                            Mark::Miss | Mark::Sunk => {
                                open = false;
                                break;
                            }
                            Mark::Hit => hits += 1,
                            Mark::Unknown => {}
                        }
                    }
                    if !open {
                        continue;
                    }
                    let weight = if hits == 0 { 1 } else { ACROSS_HITS * hits };
                    for i in 0..len {
                        if sight.marks[cell(i)] == Mark::Unknown {
                            density[cell(i)] += weight;
                        }
                    }
                }
            }
        }
    }
    density
}

/// The squares target mode proposes: the ends of each straight run of hits,
/// the four neighbours of each isolated hit; failing those, any unknown
/// neighbour of a hit. Empty when there is no hit afloat.
pub fn proposals(sight: &Sight) -> [bool; N * N] {
    let mut proposed = [false; N * N];
    let mut any_hit = false;
    let unknown = |x: isize, y: isize| (0..N as isize).contains(&x) && (0..N as isize).contains(&y) && sight.mark(x as usize, y as usize) == Mark::Unknown;
    let hit = |x: isize, y: isize| (0..N as isize).contains(&x) && (0..N as isize).contains(&y) && sight.mark(x as usize, y as usize) == Mark::Hit;
    let propose = |x: isize, y: isize, proposed: &mut [bool; N * N]| {
        if unknown(x, y) {
            proposed[y as usize * N + x as usize] = true;
        }
    };
    for i in 0..N * N {
        let (x, y) = ((i % N) as isize, (i / N) as isize);
        if !hit(x, y) {
            continue;
        }
        any_hit = true;
        let run = |dx: isize, dy: isize| {
            let (mut a, mut b) = (0, 0);
            while hit(x - (a + 1) * dx, y - (a + 1) * dy) {
                a += 1;
            }
            while hit(x + (b + 1) * dx, y + (b + 1) * dy) {
                b += 1;
            }
            (a, b)
        };
        let (left, right) = run(1, 0);
        let (up, down) = run(0, 1);
        if left + right > 0 {
            propose(x - left - 1, y, &mut proposed);
            propose(x + right + 1, y, &mut proposed);
        }
        if up + down > 0 {
            propose(x, y - up - 1, &mut proposed);
            propose(x, y + down + 1, &mut proposed);
        }
        if left + right + up + down == 0 {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                propose(x + dx, y + dy, &mut proposed);
            }
        }
    }
    if any_hit && !proposed.contains(&true) {
        // Runs blocked at both ends (two ships side by side): try beside them.
        for i in 0..N * N {
            let (x, y) = ((i % N) as isize, (i / N) as isize);
            if hit(x, y) {
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    propose(x + dx, y + dy, &mut proposed);
                }
            }
        }
    }
    proposed
}

fn target(sight: &Sight, density: &[u32; N * N], rng: &mut Rng) -> Option<usize> {
    let proposed = proposals(sight);
    densest(density, |i| proposed[i], rng).or_else(|| random_where(|i| proposed[i], rng))
}

fn hunt(level: Level, sight: &Sight, density: &[u32; N * N], rng: &mut Rng) -> Option<usize> {
    let unknown = |i: usize| sight.marks[i] == Mark::Unknown;
    let chosen = match level {
        Level::Easy => None,
        Level::Normal if rng.unit() < NORMAL_WILD => None,
        Level::Normal => densest(density, unknown, rng),
        Level::Hard => {
            let step = sight.shortest_afloat().max(1);
            densest(density, |i| unknown(i) && (i % N + i / N) % step == 0, rng).or_else(|| densest(density, unknown, rng))
        }
    };
    chosen.or_else(|| random_where(unknown, rng))
}

/// The allowed square of greatest density (more than none), ties at random.
fn densest(density: &[u32; N * N], allowed: impl Fn(usize) -> bool, rng: &mut Rng) -> Option<usize> {
    let best = (0..N * N).filter(|i| allowed(*i)).map(|i| density[i]).max().filter(|d| *d > 0)?;
    let ties = (0..N * N).filter(|i| allowed(*i) && density[*i] == best);
    let pick = rng.below(ties.clone().count());
    ties.into_iter().nth(pick)
}

fn random_where(allowed: impl Fn(usize) -> bool, rng: &mut Rng) -> Option<usize> {
    let count = (0..N * N).filter(|i| allowed(*i)).count();
    if count == 0 {
        return None;
    }
    let pick = rng.below(count);
    (0..N * N).filter(|i| allowed(*i)).nth(pick)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Board, Kind, Ship, FLEET};

    /// Play the computer against `board` until it has sunk everything,
    /// checking every shot. Returns the shots it took.
    fn play_out(level: Level, board: &mut Board, rng: &mut Rng, mut check: impl FnMut(&Sight, (usize, usize))) -> usize {
        let mut shots = 0;
        while !board.all_sunk() {
            let sight = board.sight();
            let (x, y) = choose(level, &sight, rng).expect("a square is left while a ship is afloat");
            check(&sight, (x, y));
            assert!(board.fire(x, y).is_some(), "{level:?} fired at ({x}, {y}) twice");
            shots += 1;
            assert!(shots <= N * N);
        }
        shots
    }

    #[test]
    fn it_never_fires_at_the_same_square_twice_and_always_finishes() {
        for level in Level::ALL {
            let mut total = 0;
            for seed in 0..150 {
                let mut rng = Rng::new(seed);
                let mut board = Board::random(&mut rng);
                total += play_out(level, &mut board, &mut rng, |_, _| {});
            }
            let mean = total as f32 / 150.0;
            assert!(mean < 100.0, "{level:?}: {mean}");
            eprintln!("{level:?}: {mean:.1} shots a game");
        }
    }

    #[test]
    fn the_levels_play_better_as_they_go_up() {
        let mean = |level: Level| {
            let mut total = 0;
            for seed in 0..200 {
                let mut rng = Rng::new(1000 + seed);
                let mut board = Board::random(&mut rng);
                total += play_out(level, &mut board, &mut rng, |_, _| {});
            }
            total as f32 / 200.0
        };
        let (easy, normal, hard) = (mean(Level::Easy), mean(Level::Normal), mean(Level::Hard));
        assert!(easy > normal && normal > hard, "easy {easy}, normal {normal}, hard {hard}");
    }

    #[test]
    fn it_finds_a_lone_ship_anywhere_on_the_board() {
        for level in Level::ALL {
            for kind in FLEET {
                for across in [true, false] {
                    for at in 0..N * N {
                        let ship = Ship::new(kind, at % N, at / N, across);
                        if !ship.on_board() {
                            continue;
                        }
                        let mut rng = Rng::new(at as u64 * 7 + kind.index() as u64);
                        let shots = LoneBoard::new(ship).play(level, &mut rng);
                        assert!(shots <= N * N, "{level:?} {ship:?}");
                    }
                }
            }
        }
    }

    /// One ship alone in the waters, fired at by the computer player.
    struct LoneBoard {
        ship: Ship,
        fired: [bool; N * N],
    }

    impl LoneBoard {
        fn new(ship: Ship) -> LoneBoard {
            LoneBoard { ship, fired: [false; N * N] }
        }

        fn sight(&self) -> Sight {
            let sunk = self.ship.cells().all(|(x, y)| self.fired[y * N + x]);
            let mut marks = [Mark::Unknown; N * N];
            for (i, mark) in marks.iter_mut().enumerate() {
                if self.fired[i] {
                    *mark = match (self.ship.covers(i % N, i / N), sunk) {
                        (false, _) => Mark::Miss,
                        (true, false) => Mark::Hit,
                        (true, true) => Mark::Sunk,
                    };
                }
            }
            let mut afloat = [false; FLEET.len()];
            afloat[self.ship.kind.index()] = !sunk;
            Sight { marks, afloat }
        }

        fn play(&mut self, level: Level, rng: &mut Rng) -> usize {
            let mut shots = 0;
            loop {
                let sight = self.sight();
                if !sight.afloat[self.ship.kind.index()] {
                    return shots;
                }
                let (x, y) = choose(level, &sight, rng).expect("a square is left while the ship is afloat");
                assert!(!self.fired[y * N + x], "{level:?} fired at ({x}, {y}) twice");
                self.fired[y * N + x] = true;
                shots += 1;
                assert!(shots <= N * N, "{level:?} never found {:?}", self.ship);
            }
        }
    }

    #[test]
    fn hard_hunts_on_the_checkerboard_of_the_shortest_ship_afloat() {
        let mut parity_shots = 0;
        for seed in 0..80 {
            let mut rng = Rng::new(seed);
            let mut board = Board::random(&mut rng);
            play_out(Level::Hard, &mut board, &mut rng, |sight, (x, y)| {
                let hunting = !sight.marks.contains(&Mark::Hit);
                if !hunting {
                    return;
                }
                let step = sight.shortest_afloat();
                let density = density(sight);
                let open_on_parity = (0..N * N).any(|i| sight.marks[i] == Mark::Unknown && (i % N + i / N) % step == 0 && density[i] > 0);
                if open_on_parity {
                    assert_eq!((x + y) % step, 0, "hunting off the checkerboard of {step} at ({x}, {y})");
                    parity_shots += 1;
                }
            });
        }
        assert!(parity_shots > 500, "{parity_shots}");
        // With the destroyer gone, the checkerboard opens out to every third square.
        let mut board = Board::new();
        for ship in [
            Ship::new(Kind::Carrier, 0, 9, true),
            Ship::new(Kind::Battleship, 6, 9, true),
            Ship::new(Kind::Cruiser, 9, 0, false),
            Ship::new(Kind::Submarine, 9, 4, false),
            Ship::new(Kind::Destroyer, 0, 0, true),
        ] {
            assert!(board.place(ship));
        }
        board.fire(0, 0);
        board.fire(1, 0);
        for seed in 0..50 {
            let (x, y) = choose(Level::Hard, &board.sight(), &mut Rng::new(seed)).unwrap();
            assert_eq!((x + y) % 3, 0, "({x}, {y})");
        }
    }

    #[test]
    fn target_mode_fires_beside_an_isolated_hit() {
        for level in Level::ALL {
            for seed in 0..60 {
                let mut rng = Rng::new(seed);
                let mut board = Board::random(&mut rng);
                // Hit the middle of the battleship, alone.
                let ship = board.ships()[1];
                let (hx, hy) = ship.cells().nth(1).unwrap();
                board.fire(hx, hy);
                let (x, y) = choose(level, &board.sight(), &mut rng).unwrap();
                assert_eq!(x.abs_diff(hx) + y.abs_diff(hy), 1, "{level:?}: ({x}, {y}) is not beside ({hx}, {hy})");
            }
        }
    }

    #[test]
    fn target_mode_follows_a_run_of_hits_to_its_ends() {
        for level in Level::ALL {
            for seed in 0..60 {
                let mut rng = Rng::new(seed);
                let mut board = Board::new();
                board.place(Ship::new(Kind::Carrier, 3, 4, true));
                board.place_rest_at_random(&mut rng);
                board.fire(4, 4);
                board.fire(5, 4);
                let (x, y) = choose(level, &board.sight(), &mut rng).unwrap();
                assert!(y == 4 && (x == 3 || x == 6), "{level:?}: ({x}, {y})");
            }
        }
    }
}
