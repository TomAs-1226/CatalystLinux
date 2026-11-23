//! Joining with a six-digit code, for a handheld that cannot scan the QR
//! code: Catalyst Companion's pairing (SPAKE2, companion-proto's `pair`),
//! with Team's own names for the two ends.
//!
//! 1. A member opens Invite: it shows the team's QR code and a random
//!    six-digit code, good for five minutes and five tries.
//! 2. The joiner types the code. It broadcasts SPAKE2's first message made
//!    from it (an *ask*, `CJ`, on Team's port), never the code.
//! 3. The inviting handheld answers each ask (each one a try) with its own
//!    SPAKE2 message and the team's key and name, sealed (XChaCha20-Poly1305)
//!    under a key derived from what SPAKE2 agreed. With the wrong code the
//!    two ends agree on different keys, and the seal does not open.
//!
//! Someone listening learns nothing to test codes against; someone asking
//! in the joiner's place gets one guess a try: five in a million for the
//! code's life, after which Invite shows a new one.

use catalyst_companion_proto::pair;

use super::crypto::{self, TeamKey, KEY_LEN};

pub const MAGIC: &[u8; 2] = b"CJ";
const VERSION: u8 = 1;
const ASK: u8 = 1;
const OFFER: u8 = 2;
/// The two ends' names in SPAKE2: not Companion's.
const JOINER: &[u8] = b"catalyst-team joiner";
const MEMBER: &[u8] = b"catalyst-team member";
/// A code's life, in milliseconds.
pub const LIFETIME: u64 = catalyst_companion_proto::CODE_LIFETIME * 1000;
/// Asks a code answers.
pub const TRIES: u32 = catalyst_companion_proto::TRIES;

/// A new code: six digits, evenly drawn.
pub fn new_code() -> Option<String> {
    loop {
        let mut b = [0u8; 4];
        if !crypto::random(&mut b) {
            return None;
        }
        let n = u32::from_le_bytes(b);
        // Below the last whole million, so every code is as likely.
        if n < u32::MAX - u32::MAX % 1_000_000 {
            return Some(format!("{:06}", n % 1_000_000));
        }
    }
}

/// The key a join's seal is under, from what SPAKE2 agreed.
fn seal_key(agreed: &[u8]) -> zeroize::Zeroizing<[u8; KEY_LEN]> {
    crypto::derive(b"catalyst-team join", agreed)
}

/// A member's invitation, while Invite shows.
pub struct Invite {
    pub code: String,
    made: u64,
    /// Asks answered: each one's id and the answer (sent again, unchanged,
    /// to the same ask resent, without costing a try).
    answered: Vec<([u8; 8], Vec<u8>)>,
}

impl Invite {
    pub fn new(now: u64) -> Option<Invite> {
        Some(Invite { code: new_code()?, made: now, answered: Vec::new() })
    }

    /// Spent: its time is up, or its tries.
    pub fn spent(&self, now: u64) -> bool {
        now.saturating_sub(self.made) >= LIFETIME || self.answered.len() as u32 >= TRIES
    }

    /// Seconds left.
    pub fn left(&self, now: u64) -> u64 {
        LIFETIME.saturating_sub(now.saturating_sub(self.made)).div_ceil(1000)
    }

    /// The answer to an ask: SPAKE2's second message and the team's key and
    /// name, sealed. None for what is not an ask, or once spent.
    pub fn answer(&mut self, packet: &[u8], key: &TeamKey, team: &str, now: u64) -> Option<Vec<u8>> {
        let (id, message) = read_ask(packet)?;
        if let Some((_, again)) = self.answered.iter().find(|(a, _)| *a == id) {
            return Some(again.clone());
        }
        if self.spent(now) {
            return None;
        }
        let (theirs, agreed) = pair::answer_as(&self.code, JOINER, MEMBER, message).ok()?;
        let mut out = vec![MAGIC[0], MAGIC[1], VERSION, OFFER];
        out.extend_from_slice(&id);
        out.push(theirs.len() as u8);
        out.extend_from_slice(&theirs);
        let mut plain = zeroize::Zeroizing::new(key.bytes().to_vec());
        plain.extend_from_slice(team.as_bytes());
        let sealed = crypto::seal_with(&seal_key(&agreed), &out, &plain)?;
        out.extend(sealed);
        self.answered.push((id, out.clone()));
        Some(out)
    }
}

fn read_ask(p: &[u8]) -> Option<([u8; 8], &[u8])> {
    if p.len() < 13 || &p[..2] != MAGIC || p[2] != VERSION || p[3] != ASK {
        return None;
    }
    let id: [u8; 8] = p[4..12].try_into().ok()?;
    let n = p[12] as usize;
    (p.len() == 13 + n).then(|| (id, &p[13..]))
}

/// Whether a packet is a join's at all.
pub fn is_join(p: &[u8]) -> bool {
    p.len() > 4 && &p[..2] == MAGIC
}

/// The joiner's half: its ask, and what it needs to read the answer.
pub struct Asking {
    id: [u8; 8],
    state: pair::Computer,
}

impl Asking {
    /// An ask made from the code typed: this end, and the packet to send
    /// (again and again until answered).
    pub fn new(code: &str) -> Option<(Asking, Vec<u8>)> {
        let mut id = [0u8; 8];
        if !crypto::random(&mut id) {
            return None;
        }
        let (state, message) = pair::Computer::start_as(code, JOINER, MEMBER);
        let mut out = vec![MAGIC[0], MAGIC[1], VERSION, ASK];
        out.extend_from_slice(&id);
        out.push(message.len() as u8);
        out.extend_from_slice(&message);
        Some((Asking { id, state }, out))
    }

    /// Whether a packet is the answer to this ask.
    pub fn answers(&self, p: &[u8]) -> bool {
        p.len() > 13 && &p[..2] == MAGIC && p[2] == VERSION && p[3] == OFFER && p[4..12] == self.id
    }

    /// The team's key and name, if the code was the one shown.
    pub fn finish(self, p: &[u8]) -> Option<(TeamKey, String)> {
        if !self.answers(p) {
            return None;
        }
        let n = p[12] as usize;
        let head = 13 + n;
        if p.len() < head {
            return None;
        }
        let agreed = zeroize::Zeroizing::new(self.state.finish(&p[13..head]).ok()?);
        let plain = zeroize::Zeroizing::new(crypto::open_with(&seal_key(&agreed), &p[..head], &p[head..])?);
        if plain.len() < KEY_LEN {
            return None;
        }
        let key: [u8; KEY_LEN] = plain[..KEY_LEN].try_into().ok()?;
        let team = String::from_utf8(plain[KEY_LEN..].to_vec()).ok()?;
        Some((TeamKey::from_bytes(key), super::wire::clean_name(&team)))
    }
}

/// A code as typed: its six digits, spaces and dashes let go.
pub fn clean_code(typed: &str) -> Option<String> {
    catalyst_companion_proto::clean_code(typed)
}
