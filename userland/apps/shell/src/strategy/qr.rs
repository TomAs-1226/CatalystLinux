//! A board as QR codes, for another handheld's camera to scan: its compact
//! bytes (`board.rs`) in Base45 (RFC 9285), which a QR code holds in its
//! alphanumeric mode at 5.5 bits a character, cut into codes small enough
//! to stay version 6 (41 modules, three pixels each on this panel) at
//! error correction M.
//!
//! Each code says what it is a part of: `ST1:<board>:<k>/<n>:<data>`, the
//! board a 16-bit check of all its bytes, so codes of two boards never mix
//! and a board put together is the one that was sent. Scanned in any order.

use std::sync::Mutex;

use super::board::Board;

pub const PREFIX: &str = "ST1:";
/// Bytes a code carries: 135 Base45 characters and a 13-character head,
/// within version 6-M's 154.
pub const PER_CODE: usize = 90;
pub const MOST_CODES: usize = 9;

const ALPHABET: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

pub fn encode45(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3 / 2 + 2);
    for pair in bytes.chunks(2) {
        let (mut n, digits) = match pair {
            [a, b] => ((*a as u32) << 8 | *b as u32, 3),
            [a] => (*a as u32, 2),
            _ => unreachable!(),
        };
        for _ in 0..digits {
            out.push(ALPHABET[(n % 45) as usize] as char);
            n /= 45;
        }
    }
    out
}

pub fn decode45(text: &str) -> Option<Vec<u8>> {
    let values: Vec<u32> = text.bytes().map(|c| ALPHABET.iter().position(|a| *a == c).map(|p| p as u32)).collect::<Option<_>>()?;
    let mut out = Vec::with_capacity(values.len() * 2 / 3 + 1);
    for group in values.chunks(3) {
        match group {
            [c, d, e] => {
                let n = c + d * 45 + e * 45 * 45;
                if n > 0xffff {
                    return None;
                }
                out.extend_from_slice(&[(n >> 8) as u8, n as u8]);
            }
            [c, d] => {
                let n = c + d * 45;
                if n > 0xff {
                    return None;
                }
                out.push(n as u8);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// FNV-1a folded to 16 bits: the board's mark in each of its codes.
fn check(bytes: &[u8]) -> u16 {
    let h = bytes.iter().fold(0x811c_9dc5u32, |h, b| (h ^ *b as u32).wrapping_mul(0x0100_0193));
    (h >> 16) as u16 ^ h as u16
}

/// The codes' texts for a board, or why it cannot be sent.
pub fn chunks(board: &Board) -> Result<Vec<String>, String> {
    let bytes = board.encode();
    let parts: Vec<&[u8]> = bytes.chunks(PER_CODE).collect();
    if parts.len() > MOST_CODES {
        return Err("too much for codes: clear a phase first".into());
    }
    let id = check(&bytes);
    Ok(parts.iter().enumerate().map(|(k, part)| format!("{PREFIX}{id:04X}:{}/{}:{}", k + 1, parts.len(), encode45(part))).collect())
}

/// One code read.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub id: u16,
    /// Which of how many, from 1.
    pub k: usize,
    pub n: usize,
    pub data: Vec<u8>,
}

pub fn parse(text: &str) -> Option<Chunk> {
    let rest = text.trim().strip_prefix(PREFIX)?;
    let mut parts = rest.splitn(3, ':');
    let id = u16::from_str_radix(parts.next()?, 16).ok()?;
    let (k, n) = parts.next()?.split_once('/')?;
    let (k, n): (usize, usize) = (k.parse().ok()?, n.parse().ok()?);
    if k == 0 || k > n || n > MOST_CODES {
        return None;
    }
    Some(Chunk { id, k, n, data: decode45(parts.next()?)? })
}

/// Codes of one board, as they are scanned.
#[derive(Debug, Default)]
pub struct Gather {
    id: u16,
    parts: Vec<Option<Vec<u8>>>,
}

/// How a scan left things.
#[derive(Debug)]
pub enum Scan {
    /// Kept: so many of the board's codes now.
    Kept { have: usize, of: usize },
    Done(Box<Board>),
    /// All there, but not a board this handheld can read.
    Broken,
}

impl Gather {
    pub fn add(&mut self, chunk: Chunk) -> Scan {
        if chunk.id != self.id || chunk.n != self.parts.len() {
            *self = Gather { id: chunk.id, parts: vec![None; chunk.n] };
        }
        self.parts[chunk.k - 1] = Some(chunk.data);
        let have = self.parts.iter().filter(|p| p.is_some()).count();
        if have < self.parts.len() {
            return Scan::Kept { have, of: self.parts.len() };
        }
        let bytes: Vec<u8> = self.parts.iter().flatten().flatten().copied().collect();
        *self = Gather::default();
        match Board::decode(&bytes).filter(|_| check(&bytes) == chunk.id) {
            Some(board) => Scan::Done(Box::new(board)),
            None => Scan::Broken,
        }
    }
}

/// What the camera has gathered so far, across scans.
static GATHERED: Mutex<Option<Gather>> = Mutex::new(None);

/// A code the camera read, if it is one of a board's.
pub fn scanned(text: &str) -> Option<Scan> {
    let chunk = parse(text)?;
    let mut g = GATHERED.lock().unwrap_or_else(|e| e.into_inner());
    Some(g.get_or_insert_with(Gather::default).add(chunk))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::board::{tests::full, Note, NoteKind, Phase, Route};

    #[test]
    fn base45_is_rfc_9285s() {
        // The RFC's own examples.
        assert_eq!(encode45(b"AB"), "BB8");
        assert_eq!(encode45(b"Hello!!"), "%69 VD92EX0");
        assert_eq!(encode45(b"base-45"), "UJCLQE7W581");
        assert_eq!(decode45("QED8WEX0").as_deref(), Some(&b"ietf!"[..]));
        assert_eq!(decode45("GGW"), None, "65535 is the most three can say");
        assert_eq!(decode45("abc"), None, "lower case is not in it");
        assert_eq!(decode45("A"), None);
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode45(&encode45(&all)).unwrap(), all);
    }

    #[test]
    fn a_board_goes_through_its_codes_and_comes_back_whole_in_any_order() {
        let mut board = full();
        // Busy enough to need several codes.
        for k in 0..6 {
            board.layer_mut(Phase::Teleop).paths.push(Route { robot: k % 6, points: (0..5).map(|i| (2.0 + i as f32 * 2.1, 1.0 + k as f32)).collect() });
            board.layer_mut(Phase::Endgame).notes.push(Note { kind: NoteKind::Label, robot: k, at: (8.0, 1.0 + k as f32), text: format!("climb {k} at 0:{:02}", 20 + k) });
        }
        let codes = chunks(&board).unwrap();
        assert!(codes.len() >= 3, "{} codes", codes.len());
        for code in &codes {
            // Alphanumeric mode, within version 6 at M.
            assert!(code.len() <= 154, "{}", code.len());
            assert!(code.bytes().all(|c| ALPHABET.contains(&c)));
            let qr = qrcodegen::QrCode::encode_text(code, qrcodegen::QrCodeEcc::Medium).unwrap();
            assert!(qr.size() <= 41, "version {}", (qr.size() - 17) / 4);
        }
        let mut gather = Gather::default();
        let mut order: Vec<&String> = codes.iter().rev().collect();
        // A code of another board in among them starts that one afresh.
        let other = chunks(&crate::strategy::board::Board::practice(Some(1))).unwrap();
        assert!(matches!(gather.add(parse(&other[0]).unwrap()), Scan::Done(_)), "one code is a whole small board");
        // Seen twice: still the one.
        order.insert(1, order[0]);
        let mut done = None;
        for (i, code) in order.iter().enumerate() {
            match gather.add(parse(code).unwrap()) {
                Scan::Kept { have, of } => assert!(have < of && of == codes.len() && i < order.len() - 1),
                Scan::Done(b) => done = Some(b),
                Scan::Broken => panic!("broken"),
            }
        }
        assert_eq!(done.expect("whole").encode(), board.encode());
    }

    #[test]
    fn a_code_that_is_not_a_boards_is_left_alone() {
        assert_eq!(parse("WIFI:T:WPA;S:Pit;;"), None);
        assert_eq!(parse("ST1:ZZZZ:1/2:BB8"), None);
        assert_eq!(parse("ST1:00AF:3/2:BB8"), None);
        assert_eq!(parse("ST1:00AF:1/2:bb8"), None);
        assert!(parse("ST1:00AF:1/2:BB8").is_some());
        // All its codes, and the check does not hold: broken, not a wrong board.
        let mut gather = Gather::default();
        assert!(matches!(gather.add(parse("ST1:00AF:1/1:BB8").unwrap()), Scan::Broken));
    }
}
