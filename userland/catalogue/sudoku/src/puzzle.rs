//! Making a puzzle with exactly one answer, as sudokuzero makes them: fill
//! a whole grid by backtracking over shuffled digits (bit masks for each
//! row, column and box), then visit the cells in a random order and empty
//! each one only if the puzzle still has a single solution, until the
//! level's number of givens is left.

use catalyst_kit::Rng;

/// The three levels: how many digits are given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Easy,
    Standard,
    Hard,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Easy, Level::Standard, Level::Hard];

    pub fn givens(self) -> usize {
        match self {
            Level::Easy => 40,
            Level::Standard => 32,
            Level::Hard => 26,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Level::Easy => "easy",
            Level::Standard => "standard",
            Level::Hard => "hard",
        }
    }

    pub fn from_name(name: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|l| l.name() == name)
    }
}

pub fn row_of(i: usize) -> usize {
    i / 9
}
pub fn col_of(i: usize) -> usize {
    i % 9
}
pub fn box_of(i: usize) -> usize {
    (i / 27) * 3 + (i % 9) / 3
}

/// The digits a cell can still take, as bits 1..=9, given the others.
fn candidates(grid: &[u8; 81], at: usize) -> u16 {
    let mut used = 0u16;
    let (r, c, b) = (row_of(at), col_of(at), box_of(at));
    for i in 0..81 {
        if grid[i] != 0 && i != at && (row_of(i) == r || col_of(i) == c || box_of(i) == b) {
            used |= 1 << grid[i];
        }
    }
    !used & 0b11_1111_1110
}

/// How many solutions the grid has, counting no further than `limit`.
/// Fills the emptiest-choice cell first, which keeps it quick.
pub fn count_solutions(grid: &mut [u8; 81], limit: usize) -> usize {
    let mut best: Option<(usize, u16)> = None;
    for i in 0..81 {
        if grid[i] == 0 {
            let options = candidates(grid, i);
            if options == 0 {
                return 0;
            }
            if best.is_none_or(|(_, b)| options.count_ones() < b.count_ones()) {
                best = Some((i, options));
                if options.count_ones() == 1 {
                    break;
                }
            }
        }
    }
    let Some((at, options)) = best else { return 1 };
    let mut found = 0;
    for d in 1..=9u8 {
        if options & (1 << d) != 0 {
            grid[at] = d;
            found += count_solutions(grid, limit - found);
            grid[at] = 0;
            if found >= limit {
                break;
            }
        }
    }
    found
}

/// A whole, valid grid, from shuffled digits.
fn fill(grid: &mut [u8; 81], at: usize, rng: &mut Rng) -> bool {
    if at == 81 {
        return true;
    }
    let options = candidates(grid, at);
    let mut digits: Vec<u8> = (1..=9).filter(|d| options & (1 << d) != 0).collect();
    rng.shuffle(&mut digits);
    for d in digits {
        grid[at] = d;
        if fill(grid, at + 1, rng) {
            return true;
        }
    }
    grid[at] = 0;
    false
}

/// A puzzle and its answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Puzzle {
    pub givens: [u8; 81],
    pub solution: [u8; 81],
}

/// Make a puzzle at a level: one answer, and as close to the level's givens
/// as the grid allows (every cell is tried once).
pub fn make(level: Level, rng: &mut Rng) -> Puzzle {
    let mut solution = [0u8; 81];
    fill(&mut solution, 0, rng);
    let mut givens = solution;
    let mut order: Vec<usize> = (0..81).collect();
    rng.shuffle(&mut order);
    let mut left = 81;
    for at in order {
        if left <= level.givens() {
            break;
        }
        let kept = givens[at];
        givens[at] = 0;
        let mut trial = givens;
        if count_solutions(&mut trial, 2) == 1 {
            left -= 1;
        } else {
            givens[at] = kept;
        }
    }
    Puzzle { givens, solution }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(grid: &[u8; 81]) -> bool {
        (0..81).all(|i| grid[i] != 0 && candidates(grid, i) & (1 << grid[i]) != 0)
    }

    #[test]
    fn every_level_has_one_answer_and_its_givens() {
        let mut rng = Rng::new(42);
        for level in Level::ALL {
            let p = make(level, &mut rng);
            assert!(valid(&p.solution), "{level:?}: the answer is a whole sudoku");
            let given = p.givens.iter().filter(|d| **d != 0).count();
            assert!(given >= level.givens() && given <= level.givens() + 4, "{level:?}: {given} given");
            assert!((0..81).all(|i| p.givens[i] == 0 || p.givens[i] == p.solution[i]));
            let mut g = p.givens;
            assert_eq!(count_solutions(&mut g, 2), 1, "{level:?}: exactly one answer");
        }
    }

    #[test]
    fn a_grid_with_two_answers_is_counted_so() {
        let mut rng = Rng::new(7);
        let p = make(Level::Easy, &mut rng);
        let mut open = [0u8; 81];
        // Only the first three rows kept: many answers.
        open[..27].copy_from_slice(&p.solution[..27]);
        assert_eq!(count_solutions(&mut open, 2), 2);
        let mut whole = p.solution;
        assert_eq!(count_solutions(&mut whole, 2), 1);
        assert_eq!((box_of(0), box_of(80), box_of(30)), (0, 8, 4));
    }
}
