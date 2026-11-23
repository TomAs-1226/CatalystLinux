//! What one member does with what it hears and what it says, with no
//! socket of its own: given packets and the time, it answers with packets
//! (net.rs sends them; the tests pass them round a pretend network).
//!
//! - **Discovery.** Each member says hello to everyone every
//!   [`HELLO_EVERY`], and at once on starting and on hearing someone new
//!   or back. A member not heard for [`GONE_AFTER`] has gone.
//! - **Messages** have an id (their sender and its count), a Lamport time
//!   and the sender's clock. Everyone shows them in Lamport order, ties
//!   broken by sender, so the thread reads the same on every handheld
//!   whatever order the packets came in.
//! - **Resent until acknowledged.** A message goes out to everyone at once,
//!   and again after 0.5, 1, 2, 4 and then every 8 s while a member who was
//!   there when it was sent has not acknowledged it (and is still there),
//!   up to [`TRIES`] times.
//! - **Each once.** What has been had is kept as, per sender, the count up
//!   to which all have come, and the few beyond it: a resend, a replay or a
//!   copy by both broadcast and multicast is acknowledged again and
//!   otherwise let go.
//! - **Catching up.** A hello says how much of each sender's messages its
//!   member has. Whoever has what it lacks among the last [`KEEP`] kept
//!   sends it on, `late`: the lowest-numbered member who has each message,
//!   so ten handhelds do not all answer; everyone who has it if the member
//!   still lacks it at its next hello. Messages let go of everywhere are
//!   passed over with a floor.
//! - **Replays.** Messages and acknowledgements count once however often
//!   they come. Hellos and goodbyes are taken only newer than the last from
//!   their sender (its boot, and the count within it), so an old one played
//!   back cannot bring a member back or put back what it had read.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;

use serde_json::{json, Value};

use super::crypto::TeamKey;
use super::wire::{self, Body, Frame, Head, MemberId, Message, MsgId, ShareKind};

/// Messages kept, and so handed on to a member catching up.
pub const KEEP: usize = 200;
/// Hellos, in milliseconds.
pub const HELLO_EVERY: u64 = 12_000;
/// Not heard for this long: gone.
pub const GONE_AFTER: u64 = 40_000;
/// The first resend, doubling to [`MOST_RETRY`].
pub const FIRST_RETRY: u64 = 500;
pub const MOST_RETRY: u64 = 8_000;
/// Sends of a message (the first and its resends), at most.
pub const TRIES: u32 = 10;
/// Boots of a member remembered as over (for its old hellos).
const OLD_BOOTS: usize = 8;

/// Where a packet goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum To {
    /// Everyone: broadcast and multicast.
    All,
    /// One member, at the address it was last heard from.
    Peer(MemberId),
}

pub type Out = Vec<(To, Vec<u8>)>;

/// What happened, for the island and the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A message from someone else: as it was sent, or `late`, missed while
    /// away and handed on.
    Arrived { m: Message, late: bool },
    /// Someone appeared: first seen, or back.
    Here(MemberId),
    Gone(MemberId),
}

/// How much of one member's messages are had: all up to `upto`, and these
/// beyond it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Have {
    pub upto: u32,
    pub extra: BTreeSet<u32>,
}

impl Have {
    pub fn has(&self, seq: u32) -> bool {
        seq <= self.upto || self.extra.contains(&seq)
    }

    pub fn add(&mut self, seq: u32) {
        if !self.has(seq) {
            self.extra.insert(seq);
            self.settle();
        }
    }

    /// Everything to `seq` counts as had (it was let go of everywhere).
    pub fn floor(&mut self, seq: u32) {
        if seq > self.upto {
            self.upto = seq;
            self.settle();
        }
    }

    fn settle(&mut self) {
        let upto = self.upto;
        self.extra.retain(|&s| s > upto);
        while self.extra.remove(&(self.upto + 1)) {
            self.upto += 1;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Peer {
    pub name: String,
    pub addr: Option<SocketAddr>,
    /// When last heard, in the engine's milliseconds (0: not this time).
    pub heard: u64,
    /// The Lamport time it has read to.
    pub seen: u64,
    /// How much of each member's messages it has, by its last hello.
    pub have: BTreeMap<MemberId, u32>,
    boot: u32,
    count: u32,
    old_boots: Vec<u32>,
    /// Hellos in a row that showed it lacking something.
    lacks: u8,
}

impl Peer {
    pub fn here(&self, now: u64) -> bool {
        self.heard > 0 && now.saturating_sub(self.heard) < GONE_AFTER
    }

    /// A hello or goodbye newer than the last from it (and so not a replay).
    fn fresh(&mut self, head: &Head) -> bool {
        if head.boot == self.boot {
            if head.count <= self.count {
                return false;
            }
        } else if self.old_boots.contains(&head.boot) {
            return false;
        } else {
            if self.boot != 0 {
                self.old_boots.push(self.boot);
                if self.old_boots.len() > OLD_BOOTS {
                    self.old_boots.remove(0);
                }
            }
            self.boot = head.boot;
        }
        self.count = head.count;
        true
    }
}

/// A message kept, and who is known to have it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub m: Message,
    pub got: BTreeSet<MemberId>,
}

struct Pending {
    id: MsgId,
    waiting: BTreeSet<MemberId>,
    tries: u32,
    at: u64,
}

pub struct Engine {
    pub key: TeamKey,
    pub me: MemberId,
    pub name: String,
    boot: u32,
    count: u32,
    pub clock: u64,
    pub next_seq: u32,
    /// The thread, in order, at most [`KEEP`].
    pub log: Vec<Entry>,
    pub have: BTreeMap<MemberId, Have>,
    pub peers: BTreeMap<MemberId, Peer>,
    outbox: Vec<Pending>,
    acks: Vec<MsgId>,
    hello_at: u64,
    /// The Lamport time read to here.
    pub seen: u64,
    pub events: Vec<Event>,
    /// Something changed worth keeping or drawing.
    pub changed: bool,
}

impl Engine {
    pub fn new(key: TeamKey, me: MemberId, name: &str, boot: u32) -> Engine {
        Engine {
            key,
            me,
            name: wire::clean_name(name),
            boot: boot.max(1),
            count: 0,
            clock: 0,
            next_seq: 1,
            log: Vec::new(),
            have: BTreeMap::new(),
            peers: BTreeMap::new(),
            outbox: Vec::new(),
            acks: Vec::new(),
            hello_at: 0,
            seen: 0,
            events: Vec::new(),
            changed: false,
        }
    }

    fn pack(&mut self, frame: &Frame) -> Option<Vec<u8>> {
        self.count = self.count.wrapping_add(1);
        wire::seal(&self.key, &Head { team: self.key.id, from: self.me, boot: self.boot, count: self.count }, frame)
    }

    /// Members here now.
    pub fn here(&self, now: u64) -> impl Iterator<Item = (&MemberId, &Peer)> {
        self.peers.iter().filter(move |(_, p)| p.here(now))
    }

    pub fn has(&self, id: MsgId) -> bool {
        self.have.get(&id.origin).is_some_and(|h| h.has(id.seq))
    }

    /// The newest message's Lamport time.
    pub fn newest(&self) -> u64 {
        self.log.last().map_or(0, |e| e.m.lamport)
    }

    /// Kept in its place in the thread; the oldest let go past [`KEEP`].
    fn keep(&mut self, m: Message, got: MemberId) {
        self.have.entry(m.id.origin).or_default().add(m.id.seq);
        let at = self.log.partition_point(|e| e.m.order() < m.order());
        let mut entry = Entry { m, got: BTreeSet::new() };
        entry.got.insert(got);
        self.log.insert(at, entry);
        if self.log.len() > KEEP {
            self.log.remove(0);
        }
        self.changed = true;
    }

    /// Say something: kept, sent to everyone, and resent until those here
    /// have it.
    pub fn send(&mut self, body: Body, urgent: bool, wall: i64, now: u64) -> Out {
        self.clock += 1;
        let m = Message { id: MsgId { origin: self.me, seq: self.next_seq }, lamport: self.clock, wall, by: self.name.clone(), urgent, body };
        self.next_seq += 1;
        self.seen = self.seen.max(m.lamport);
        self.keep(m.clone(), self.me);
        let waiting: BTreeSet<MemberId> = self.here(now).map(|(id, _)| *id).collect();
        if !waiting.is_empty() {
            self.outbox.push(Pending { id: m.id, waiting, tries: 1, at: now + FIRST_RETRY });
        }
        self.pack(&Frame::Msg { m, late: false }).map(|p| vec![(To::All, p)]).unwrap_or_default()
    }

    /// Read to the newest: said in the next hello, soon.
    pub fn read_all(&mut self, now: u64) {
        let newest = self.newest();
        if newest > self.seen {
            self.seen = newest;
            self.hello_at = self.hello_at.min(now + 300);
            self.changed = true;
        }
    }

    pub fn hello(&mut self) -> Option<Vec<u8>> {
        let have: Vec<(MemberId, u32)> = self.have.iter().map(|(m, h)| (*m, h.upto)).collect();
        self.pack(&Frame::Hello { name: self.name.clone(), clock: self.clock, seen: self.seen, have })
    }

    /// Leaving: a goodbye for everyone.
    pub fn bye(&mut self) -> Option<Vec<u8>> {
        self.pack(&Frame::Bye)
    }

    /// A packet heard, from `addr`.
    pub fn receive(&mut self, packet: &[u8], addr: Option<SocketAddr>, now: u64) -> Out {
        let mut out = Out::new();
        let Some((head, frame)) = wire::open(&self.key, packet) else { return out };
        if head.from == self.me {
            return out;
        }
        let peer = self.peers.entry(head.from).or_default();
        // Hellos and goodbyes only once, and in order.
        if matches!(frame, Frame::Hello { .. } | Frame::Bye) && !peer.fresh(&head) {
            return out;
        }
        let was_here = peer.here(now);
        if let Some(a) = addr {
            peer.addr = Some(a);
        }
        peer.heard = now.max(1);
        if !was_here && !matches!(frame, Frame::Bye) {
            self.events.push(Event::Here(head.from));
            // Hello back at once, so each learns what the other lacks.
            self.hello_at = self.hello_at.min(now + jitter(head.from ^ self.me));
            self.changed = true;
        }
        match frame {
            Frame::Hello { name, clock, seen, have } => {
                let peer = self.peers.get_mut(&head.from).expect("just added");
                let name = wire::clean_name(&name);
                if !name.is_empty() && name != peer.name {
                    peer.name = name;
                    self.changed = true;
                }
                if seen != peer.seen {
                    peer.seen = seen;
                    self.changed = true;
                }
                peer.have = have.into_iter().collect();
                self.clock = self.clock.max(clock);
                out.extend(self.catch_up(head.from, now));
            }
            Frame::Msg { m, late } => {
                let id = m.id;
                self.clock = self.clock.max(m.lamport);
                if id.origin == self.me {
                    // Its own, from before a restart: never numbered again.
                    self.next_seq = self.next_seq.max(id.seq + 1);
                }
                if self.has(id) {
                    if let Some(e) = self.log.iter_mut().find(|e| e.m.id == id) {
                        e.got.insert(head.from);
                    }
                } else {
                    let arrived = m.clone();
                    self.keep(m, head.from);
                    if let Some(e) = self.log.iter_mut().find(|e| e.m.id == id) {
                        e.got.insert(self.me);
                    }
                    if id.origin != self.me {
                        self.events.push(Event::Arrived { m: arrived, late });
                    }
                }
                if !self.acks.contains(&id) {
                    self.acks.push(id);
                }
            }
            Frame::Ack(ids) => {
                for id in ids {
                    if let Some(p) = self.outbox.iter_mut().find(|p| p.id == id) {
                        p.waiting.remove(&head.from);
                    }
                    if let Some(e) = self.log.iter_mut().find(|e| e.m.id == id) {
                        self.changed |= e.got.insert(head.from);
                    }
                }
                self.outbox.retain(|p| !p.waiting.is_empty());
            }
            Frame::Floor(floors) => {
                for (origin, seq) in floors {
                    if origin != self.me {
                        self.have.entry(origin).or_default().floor(seq);
                    }
                }
            }
            Frame::Bye => {
                if let Some(p) = self.peers.get_mut(&head.from) {
                    p.heard = 0;
                }
                for p in &mut self.outbox {
                    p.waiting.remove(&head.from);
                }
                self.outbox.retain(|p| !p.waiting.is_empty());
                if was_here {
                    self.events.push(Event::Gone(head.from));
                }
                self.changed = true;
            }
        }
        out
    }

    /// What `p` lacks, by its hello, sent on to it.
    fn catch_up(&mut self, p: MemberId, now: u64) -> Out {
        let Some(peer) = self.peers.get(&p) else { return Out::new() };
        let lacking: Vec<usize> = (0..self.log.len())
            .filter(|&i| {
                let id = self.log[i].m.id;
                id.origin != p && id.seq > peer.have.get(&id.origin).copied().unwrap_or(0) && !self.outbox.iter().any(|o| o.id == id && o.waiting.contains(&p))
            })
            .collect();
        // Messages let go of everywhere: passed over.
        let mut floors: Vec<(MemberId, u32)> = Vec::new();
        for (origin, have) in &self.have {
            if *origin == p {
                continue;
            }
            let first = self.log.iter().filter(|e| e.m.id.origin == *origin).map(|e| e.m.id.seq).min().unwrap_or(have.upto + 1);
            if peer.have.get(origin).copied().unwrap_or(0) + 1 < first && have.upto + 1 >= first {
                floors.push((*origin, first - 1));
            }
        }
        let insist = peer.lacks >= 1;
        let mine: Vec<Message> = lacking
            .iter()
            .map(|&i| &self.log[i])
            .filter(|e| {
                // The lowest-numbered member here who has it (by its hello,
                // or its acknowledgement) answers, unless the member lacked
                // something last time too.
                let id = e.m.id;
                insist
                    || !self.peers.iter().any(|(x, q)| {
                        *x != p && *x < self.me && q.here(now) && (*x == id.origin || e.got.contains(x) || q.have.get(&id.origin).is_some_and(|&s| s >= id.seq))
                    })
            })
            .map(|e| e.m.clone())
            .collect();
        let peer = self.peers.get_mut(&p).expect("looked up above");
        peer.lacks = if lacking.is_empty() { 0 } else { peer.lacks.saturating_add(1) };
        let mut out = Out::new();
        if !floors.is_empty() {
            out.extend(self.pack(&Frame::Floor(floors)).map(|b| (To::Peer(p), b)));
        }
        for m in mine {
            out.extend(self.pack(&Frame::Msg { m, late: true }).map(|b| (To::Peer(p), b)));
        }
        out
    }

    /// Time's work: acknowledgements, resends, the hello, who has gone.
    pub fn poll(&mut self, now: u64) -> Out {
        let mut out = Out::new();
        // Gone.
        let gone: Vec<MemberId> = self.peers.iter().filter(|(_, p)| p.heard > 0 && !p.here(now)).map(|(id, _)| *id).collect();
        for id in gone {
            if let Some(p) = self.peers.get_mut(&id) {
                p.heard = 0;
            }
            for p in &mut self.outbox {
                p.waiting.remove(&id);
            }
            self.events.push(Event::Gone(id));
            self.changed = true;
        }
        self.outbox.retain(|p| !p.waiting.is_empty());
        // Acknowledgements, as many to a packet as fit.
        let acks = std::mem::take(&mut self.acks);
        for chunk in acks.chunks(wire::ACKS_MOST) {
            out.extend(self.pack(&Frame::Ack(chunk.to_vec())).map(|b| (To::All, b)));
        }
        // Resends.
        let due: Vec<MsgId> = self.outbox.iter().filter(|p| p.at <= now).map(|p| p.id).collect();
        for id in due {
            let Some(m) = self.log.iter().find(|e| e.m.id == id).map(|e| e.m.clone()) else { continue };
            out.extend(self.pack(&Frame::Msg { m, late: false }).map(|b| (To::All, b)));
            if let Some(p) = self.outbox.iter_mut().find(|p| p.id == id) {
                p.tries += 1;
                p.at = now + (FIRST_RETRY << (p.tries - 1).min(4)).min(MOST_RETRY);
            }
        }
        self.outbox.retain(|p| p.tries < TRIES && self.log.iter().any(|e| e.m.id == p.id));
        // The hello.
        if now >= self.hello_at {
            out.extend(self.hello().map(|b| (To::All, b)));
            self.hello_at = now + HELLO_EVERY;
        }
        out
    }

    /// Milliseconds until [`Engine::poll`] has something to do.
    pub fn next_due(&self, now: u64) -> u64 {
        if !self.acks.is_empty() {
            return 0;
        }
        let mut at = self.hello_at;
        for p in &self.outbox {
            at = at.min(p.at);
        }
        for p in self.peers.values().filter(|p| p.heard > 0) {
            at = at.min(p.heard + GONE_AFTER);
        }
        at.saturating_sub(now)
    }

    /// Messages still being resent.
    #[cfg(test)]
    pub fn sending(&self) -> usize {
        self.outbox.len()
    }

    /// Whether a message is still being resent (someone here lacks it).
    pub fn is_sending(&self, id: MsgId) -> bool {
        self.outbox.iter().any(|p| p.id == id)
    }

    // ------------------------------------------------------------ kept ---

    /// The thread and what it knows, as kept on the card.
    pub fn to_json(&self) -> Value {
        json!({
            "clock": self.clock,
            "next": self.next_seq,
            "seen": self.seen,
            "have": self.have.iter().map(|(m, h)| json!([m.to_string(), h.upto, h.extra.iter().collect::<Vec<_>>()])).collect::<Vec<_>>(),
            "peers": self.peers.iter().map(|(m, p)| json!({"id": m.to_string(), "name": p.name, "seen": p.seen})).collect::<Vec<_>>(),
            "log": self.log.iter().map(|e| message_json(&e.m, &e.got)).collect::<Vec<_>>(),
        })
    }

    /// What [`Engine::to_json`] kept, taken back.
    pub fn restore(&mut self, v: &Value) {
        let u = |v: &Value| v.as_u64().unwrap_or(0);
        self.clock = u(&v["clock"]);
        self.next_seq = u(&v["next"]).max(1) as u32;
        self.seen = u(&v["seen"]);
        for h in v["have"].as_array().into_iter().flatten() {
            let Some(m) = h[0].as_str().and_then(|s| s.parse().ok()) else { continue };
            let extra = h[2].as_array().into_iter().flatten().filter_map(|s| s.as_u64()).map(|s| s as u32).collect();
            self.have.insert(m, Have { upto: u(&h[1]) as u32, extra });
        }
        for p in v["peers"].as_array().into_iter().flatten() {
            let Some(m) = p["id"].as_str().and_then(|s| s.parse().ok()) else { continue };
            self.peers.insert(m, Peer { name: p["name"].as_str().unwrap_or("").to_owned(), seen: u(&p["seen"]), ..Peer::default() });
        }
        self.log = v["log"].as_array().into_iter().flatten().filter_map(message_from).collect();
        self.log.sort_by_key(|e| e.m.order());
        let own = self.have.get(&self.me).map_or(0, |h| h.upto);
        self.next_seq = self.next_seq.max(own + 1);
    }
}

/// A few hundred milliseconds, different for each pair, so hellos back do
/// not all land at once.
fn jitter(x: u64) -> u64 {
    100 + (x.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 56) * 2
}

fn message_json(m: &Message, got: &BTreeSet<MemberId>) -> Value {
    let mut v = json!({
        "o": m.id.origin.to_string(), "s": m.id.seq, "l": m.lamport, "w": m.wall, "by": m.by, "u": m.urgent,
        "got": got.iter().map(|g| g.to_string()).collect::<Vec<_>>(),
    });
    match &m.body {
        Body::Call(t) => v["call"] = json!(t),
        Body::Text(t) => v["text"] = json!(t),
        Body::Share { kind, set, part, of, label, data } => {
            v["share"] = json!({"kind": *kind as u8, "set": set, "part": part, "of": of, "label": label, "data": data});
        }
    }
    v
}

fn message_from(v: &Value) -> Option<Entry> {
    let body = if let Some(t) = v["call"].as_str() {
        Body::Call(t.to_owned())
    } else if let Some(t) = v["text"].as_str() {
        Body::Text(t.to_owned())
    } else {
        let s = &v["share"];
        let kind = if s["kind"].as_u64()? == 1 { ShareKind::Scout } else { ShareKind::Strategy };
        Body::Share {
            kind,
            set: s["set"].as_u64()? as u32,
            part: s["part"].as_u64()? as u8,
            of: s["of"].as_u64()? as u8,
            label: s["label"].as_str()?.to_owned(),
            data: s["data"].as_str()?.to_owned(),
        }
    };
    let id = MsgId { origin: v["o"].as_str()?.parse().ok()?, seq: v["s"].as_u64()? as u32 };
    let m = Message { id, lamport: v["l"].as_u64()?, wall: v["w"].as_i64()?, by: v["by"].as_str()?.to_owned(), urgent: v["u"].as_bool()?, body };
    let got = v["got"].as_array().into_iter().flatten().filter_map(|g| g.as_str()?.parse().ok()).collect();
    Some(Entry { m, got })
}
