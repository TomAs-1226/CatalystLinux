//! What crosses the network, byte for byte.
//!
//! A packet, all integers little-endian:
//!
//! | at | bytes | what |
//! | --- | --- | --- |
//! | 0 | 2 | `CT` |
//! | 2 | 1 | the format's version, 1 |
//! | 3 | 8 | the team's public id (crypto.rs) |
//! | 11 | 8 | the sender: its member id |
//! | 19 | 4 | the sender's boot: drawn at random each time Team starts |
//! | 23 | 4 | the packet's count within that boot |
//! | 27 | 24 | the nonce |
//! | 51 | … | the frame, sealed, and its 16-byte tag; bytes 0–27 its associated data |
//!
//! A packet is at most [`MOST`] bytes, under any Wi-Fi's MTU, so it is
//! never split into fragments (one lost fragment loses the lot).
//!
//! The frames inside: a hello (name, Lamport clock, how far its owner has
//! read, and how much of each member's messages it has), a message (as
//! sent, or `late`: handed on to a member catching up), acknowledgements,
//! floors (a member's messages before this were let go of everywhere),
//! and goodbye.

use super::crypto::{TeamKey, OVERHEAD};

pub const MAGIC: &[u8; 2] = b"CT";
pub const VERSION: u8 = 1;
/// The header: everything before the nonce.
pub const HEAD: usize = 27;
/// A packet's most bytes.
pub const MOST: usize = 1200;
/// A name's most characters.
pub const NAME_MOST: usize = 24;
/// Free text's most characters.
pub const TEXT_MOST: usize = 200;
/// A share part's most bytes.
pub const DATA_MOST: usize = 820;
/// Acknowledgements in one packet, at most.
pub const ACKS_MOST: usize = 60;
/// Members a hello tells of, at most.
pub const HAVE_MOST: usize = 40;

pub type MemberId = u64;

/// A message's identity: who first sent it, and its number among theirs
/// (from 1). Two are the same message exactly when these are.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct MsgId {
    pub origin: MemberId,
    pub seq: u32,
}

/// What a share carries, read with that app's own decoder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShareKind {
    /// Scout's codes' texts (scout/share.rs), separated by `\x1e`.
    Scout = 1,
    /// A Strategy board's codes' texts (strategy/qr.rs), a line each.
    Strategy = 2,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// One of the canned calls, in the words it was sent with.
    Call(String),
    Text(String),
    /// Part `part` of `of` of a share (`set` tells one share's parts from
    /// another's), with what it is called.
    Share { kind: ShareKind, set: u32, part: u8, of: u8, label: String, data: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub id: MsgId,
    /// Its Lamport time: the order everyone shows it in (ties by origin).
    pub lamport: u64,
    /// When it was sent, by its sender's clock, Unix seconds (shown only).
    pub wall: i64,
    /// Its sender's name as it was sent.
    pub by: String,
    pub urgent: bool,
    pub body: Body,
}

impl Message {
    /// Where it goes in the thread.
    pub fn order(&self) -> (u64, MemberId, u32) {
        (self.lamport, self.id.origin, self.id.seq)
    }

    /// In a few words, as the island says it.
    pub fn words(&self) -> String {
        match &self.body {
            Body::Call(w) | Body::Text(w) => w.clone(),
            Body::Share { label, .. } => format!("shared {label}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    Hello { name: String, clock: u64, seen: u64, have: Vec<(MemberId, u32)> },
    Msg { m: Message, late: bool },
    Ack(Vec<MsgId>),
    Floor(Vec<(MemberId, u32)>),
    Bye,
}

/// A packet's header.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Head {
    pub team: [u8; 8],
    pub from: MemberId,
    pub boot: u32,
    pub count: u32,
}

impl Head {
    pub fn bytes(&self) -> [u8; HEAD] {
        let mut b = [0u8; HEAD];
        b[..2].copy_from_slice(MAGIC);
        b[2] = VERSION;
        b[3..11].copy_from_slice(&self.team);
        b[11..19].copy_from_slice(&self.from.to_le_bytes());
        b[19..23].copy_from_slice(&self.boot.to_le_bytes());
        b[23..27].copy_from_slice(&self.count.to_le_bytes());
        b
    }

    pub fn read(b: &[u8]) -> Option<Head> {
        if b.len() < HEAD || &b[..2] != MAGIC || b[2] != VERSION {
            return None;
        }
        Some(Head {
            team: b[3..11].try_into().ok()?,
            from: u64::from_le_bytes(b[11..19].try_into().ok()?),
            boot: u32::from_le_bytes(b[19..23].try_into().ok()?),
            count: u32::from_le_bytes(b[23..27].try_into().ok()?),
        })
    }
}

/// A frame sealed into a packet; none if it would be too long.
pub fn seal(key: &TeamKey, head: &Head, frame: &Frame) -> Option<Vec<u8>> {
    let plain = frame.encode();
    if HEAD + OVERHEAD + plain.len() > MOST {
        return None;
    }
    let mut out = head.bytes().to_vec();
    out.extend(key.seal(&out, &plain)?);
    Some(out)
}

/// A packet opened: only this team's, whole and unchanged.
pub fn open(key: &TeamKey, packet: &[u8]) -> Option<(Head, Frame)> {
    let head = Head::read(packet)?;
    if head.team != key.id || packet.len() > MOST {
        return None;
    }
    let plain = key.open(&packet[..HEAD], &packet[HEAD..])?;
    Some((head, Frame::decode(&plain)?))
}

// ---------------------------------------------------------- encoding ---

#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        let b = s.as_bytes();
        let n = b.len().min(u16::MAX as usize);
        self.0.extend_from_slice(&(n as u16).to_le_bytes());
        self.0.extend_from_slice(&b[..n]);
    }
    fn id(&mut self, id: MsgId) {
        self.u64(id.origin);
        self.u32(id.seq);
    }
    fn pairs(&mut self, v: &[(MemberId, u32)]) {
        self.u8(v.len().min(HAVE_MOST) as u8);
        for &(m, s) in v.iter().take(HAVE_MOST) {
            self.u64(m);
            self.u32(s);
        }
    }
}

struct R<'a>(&'a [u8]);

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn str(&mut self, most: usize) -> Option<String> {
        let n = u16::from_le_bytes(self.take(2)?.try_into().ok()?) as usize;
        if n > most {
            return None;
        }
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
    fn id(&mut self) -> Option<MsgId> {
        Some(MsgId { origin: self.u64()?, seq: self.u32()? })
    }
    fn pairs(&mut self) -> Option<Vec<(MemberId, u32)>> {
        let n = self.u8()? as usize;
        if n > HAVE_MOST {
            return None;
        }
        (0..n).map(|_| Some((self.u64()?, self.u32()?))).collect()
    }
}

/// Bytes a name may take, in UTF-8: four to a character at most.
const NAME_BYTES: usize = NAME_MOST * 4;

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        match self {
            Frame::Hello { name, clock, seen, have } => {
                w.u8(1);
                w.str(name);
                w.u64(*clock);
                w.u64(*seen);
                w.pairs(have);
            }
            Frame::Msg { m, late } => {
                w.u8(2);
                w.u8(*late as u8);
                w.id(m.id);
                w.u64(m.lamport);
                w.u64(m.wall as u64);
                w.str(&m.by);
                w.u8(m.urgent as u8);
                match &m.body {
                    Body::Call(t) => {
                        w.u8(1);
                        w.str(t);
                    }
                    Body::Text(t) => {
                        w.u8(2);
                        w.str(t);
                    }
                    Body::Share { kind, set, part, of, label, data } => {
                        w.u8(3);
                        w.u8(*kind as u8);
                        w.u32(*set);
                        w.u8(*part);
                        w.u8(*of);
                        w.str(label);
                        w.str(data);
                    }
                }
            }
            Frame::Ack(ids) => {
                w.u8(3);
                w.u8(ids.len().min(ACKS_MOST) as u8);
                for id in ids.iter().take(ACKS_MOST) {
                    w.id(*id);
                }
            }
            Frame::Floor(v) => {
                w.u8(4);
                w.pairs(v);
            }
            Frame::Bye => w.u8(5),
        }
        w.0
    }

    /// A frame read back; none for anything malformed or with bytes left.
    pub fn decode(bytes: &[u8]) -> Option<Frame> {
        let mut r = R(bytes);
        let frame = match r.u8()? {
            1 => Frame::Hello { name: r.str(NAME_BYTES)?, clock: r.u64()?, seen: r.u64()?, have: r.pairs()? },
            2 => {
                let late = r.u8()? != 0;
                let id = r.id()?;
                if id.seq == 0 {
                    return None;
                }
                let (lamport, wall, by, urgent) = (r.u64()?, r.u64()? as i64, r.str(NAME_BYTES)?, r.u8()? != 0);
                let body = match r.u8()? {
                    1 => Body::Call(r.str(TEXT_MOST * 4)?),
                    2 => Body::Text(r.str(TEXT_MOST * 4)?),
                    3 => {
                        let kind = match r.u8()? {
                            1 => ShareKind::Scout,
                            2 => ShareKind::Strategy,
                            _ => return None,
                        };
                        let (set, part, of) = (r.u32()?, r.u8()?, r.u8()?);
                        if part == 0 || part > of {
                            return None;
                        }
                        Body::Share { kind, set, part, of, label: r.str(NAME_BYTES * 2)?, data: r.str(DATA_MOST)? }
                    }
                    _ => return None,
                };
                Frame::Msg { m: Message { id, lamport, wall, by, urgent, body }, late }
            }
            3 => {
                let n = r.u8()? as usize;
                if n > ACKS_MOST {
                    return None;
                }
                Frame::Ack((0..n).map(|_| r.id()).collect::<Option<_>>()?)
            }
            4 => Frame::Floor(r.pairs()?),
            5 => Frame::Bye,
            _ => return None,
        };
        r.0.is_empty().then_some(frame)
    }
}

/// Words as they are sent: trimmed, on one line, at most [`TEXT_MOST`]
/// characters.
pub fn clean_text(text: &str) -> String {
    text.trim().chars().map(|c| if c.is_control() { ' ' } else { c }).take(TEXT_MOST).collect::<String>().trim().to_owned()
}

/// A name as it is kept and sent: trimmed, at most [`NAME_MOST`] characters.
pub fn clean_name(name: &str) -> String {
    name.trim().chars().filter(|c| !c.is_control()).take(NAME_MOST).collect::<String>().trim().to_owned()
}
