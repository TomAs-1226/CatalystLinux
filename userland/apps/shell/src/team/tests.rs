//! Team: the seal, the wire, the engine (each message once, in one order,
//! resent until acknowledged, handed on to whoever was away), the join by
//! code, the QR code, and members on a pretend network and on real sockets.

use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

use super::crypto::{self, TeamKey};
use super::engine::{Engine, Event, Have, Out, To, GONE_AFTER, HELLO_EVERY, KEEP};
use super::join::{Asking, Invite};
use super::wire::{self, Body, Frame, Head, Message, MsgId, ShareKind};

fn key() -> TeamKey {
    TeamKey::from_bytes([7u8; 32])
}

fn text(e: &Engine) -> Vec<String> {
    e.log.iter().map(|x| x.m.words()).collect()
}

// ------------------------------------------------------------------ seal ---

#[test]
fn the_seal_opens_with_its_key_and_nothing_else() {
    let k = key();
    let sealed = k.seal(b"head", b"robot to queue").unwrap();
    assert_eq!(k.open(b"head", &sealed).as_deref(), Some(&b"robot to queue"[..]));
    // A fresh nonce each time: the same words never seal the same.
    assert_ne!(sealed, k.seal(b"head", b"robot to queue").unwrap());
    // Another team's key.
    assert_eq!(TeamKey::from_bytes([8u8; 32]).open(b"head", &sealed), None);
    // A byte changed anywhere: the nonce, the words, the tag, the header.
    for at in [0, crypto::NONCE_LEN, sealed.len() - 1] {
        let mut bad = sealed.clone();
        bad[at] ^= 1;
        assert_eq!(k.open(b"head", &bad), None, "changed at {at}");
    }
    assert_eq!(k.open(b"heaD", &sealed), None, "the header is bound in");
    assert_eq!(k.open(b"head", &sealed[..crypto::OVERHEAD - 1]), None, "too short");
}

#[test]
fn what_is_derived_tells_nothing_of_the_key() {
    let k = key();
    assert_ne!(&k.id[..], &k.bytes()[..8]);
    assert_ne!(k.id, TeamKey::from_bytes([8u8; 32]).id);
    assert_eq!(k.id, key().id, "the same key, the same id");
}

#[test]
fn packets_open_only_whole_and_for_their_team() {
    let k = key();
    let head = Head { team: k.id, from: 42, boot: 9, count: 3 };
    let frame = Frame::Hello { name: "ana".into(), clock: 5, seen: 4, have: vec![(42, 3)] };
    let p = wire::seal(&k, &head, &frame).unwrap();
    assert_eq!(wire::open(&k, &p), Some((head, frame.clone())));
    // The sender changed in the header: the seal fails.
    let mut forged = p.clone();
    forged[11] ^= 1;
    assert_eq!(wire::open(&k, &forged), None);
    // Another team's packet is let go without opening.
    assert_eq!(wire::open(&TeamKey::from_bytes([9u8; 32]), &p), None);
    // Too long for one datagram: refused.
    let big = Frame::Msg { m: message(1, 1, 1, Body::Text("x".repeat(wire::MOST))), late: false };
    assert_eq!(wire::seal(&k, &head, &big), None);
}

// ------------------------------------------------------------------ wire ---

fn message(origin: u64, seq: u32, lamport: u64, body: Body) -> Message {
    Message { id: MsgId { origin, seq }, lamport, wall: 1_700_000_000, by: "ana".into(), urgent: false, body }
}

#[test]
fn every_frame_goes_there_and_back() {
    let share = Body::Share { kind: ShareKind::Strategy, set: 77, part: 2, of: 3, label: "board Q31".into(), data: "ST1:a\u{1e}ST1:b".into() };
    let frames = [
        Frame::Hello { name: "Ana".into(), clock: 99, seen: 12, have: vec![(1, 4), (2, 0)] },
        Frame::Msg { m: Message { urgent: true, ..message(3, 2, 8, Body::Call("robot to queue".into())) }, late: false },
        Frame::Msg { m: message(3, 3, 9, Body::Text("bumpers on · red".into())), late: true },
        Frame::Msg { m: message(4, 1, 10, share), late: false },
        Frame::Ack(vec![MsgId { origin: 3, seq: 2 }, MsgId { origin: 4, seq: 1 }]),
        Frame::Floor(vec![(3, 40)]),
        Frame::Bye,
    ];
    for f in frames {
        let bytes = f.encode();
        assert_eq!(Frame::decode(&bytes), Some(f.clone()));
        // Cut short anywhere, or with a byte too many: not read.
        for n in 0..bytes.len() {
            assert_eq!(Frame::decode(&bytes[..n]), None, "{f:?} cut at {n}");
        }
        let mut more = bytes.clone();
        more.push(0);
        assert_eq!(Frame::decode(&more), None);
    }
    // A message numbered 0, a part beyond its count: not read.
    assert_eq!(Frame::decode(&Frame::Msg { m: message(1, 0, 1, Body::Text("x".into())), late: false }.encode()), None);
    let bad = Body::Share { kind: ShareKind::Scout, set: 1, part: 3, of: 2, label: String::new(), data: String::new() };
    assert_eq!(Frame::decode(&Frame::Msg { m: message(1, 1, 1, bad), late: false }.encode()), None);
}

#[test]
fn names_and_words_are_cleaned() {
    assert_eq!(wire::clean_name("  Ana\n "), "Ana");
    assert_eq!(wire::clean_name(&"x".repeat(40)).len(), wire::NAME_MOST);
    assert_eq!(wire::clean_text("a\nb"), "a b");
}

// ------------------------------------------------------- the network ---

/// Members on a pretend network: every packet to everyone (or to one), at
/// once, with whatever `lose` says is lost.
struct Net {
    e: Vec<Engine>,
    now: u64,
    /// (from, to, packet) → lost.
    lose: Box<dyn FnMut(usize, usize, &[u8]) -> bool>,
    /// Who is cut off (away).
    away: Vec<bool>,
    delivered: usize,
}

impl Net {
    fn new(n: usize) -> Net {
        let e = (0..n).map(|i| Engine::new(key(), 100 + i as u64, ["Ana", "Ben", "Cam", "Dee"][i % 4], 1 + i as u32)).collect();
        Net { e, now: 1_000_000, lose: Box::new(|_, _, _| false), away: vec![false; n], delivered: 0 }
    }

    fn index(&self, id: u64) -> usize {
        (id - 100) as usize
    }

    fn deliver(&mut self, from: usize, out: Out) {
        let mut queue: VecDeque<(usize, Out)> = VecDeque::from([(from, out)]);
        while let Some((from, out)) = queue.pop_front() {
            if self.away[from] {
                continue;
            }
            for (to, bytes) in out {
                let targets: Vec<usize> = match to {
                    To::All => (0..self.e.len()).filter(|&t| t != from).collect(),
                    To::Peer(id) => vec![self.index(id)],
                };
                for t in targets {
                    if self.away[t] || (self.lose)(from, t, &bytes) {
                        continue;
                    }
                    self.delivered += 1;
                    let addr = SocketAddr::from(([127, 0, 0, 1], 10_000 + from as u16));
                    let reply = self.e[t].receive(&bytes, Some(addr), self.now);
                    queue.push_back((t, reply));
                }
            }
        }
    }

    fn send(&mut self, from: usize, body: Body) {
        let out = self.e[from].send(body, false, 1_700_000_000, self.now);
        self.deliver(from, out);
    }

    /// Time passes, in steps of `step` ms, each member doing its work.
    fn run(&mut self, ms: u64, step: u64) {
        let end = self.now + ms;
        while self.now < end {
            self.now += step;
            for i in 0..self.e.len() {
                if self.away[i] {
                    continue;
                }
                let out = self.e[i].poll(self.now);
                self.deliver(i, out);
            }
        }
    }
}

#[test]
fn hellos_find_everyone_and_goodbyes_and_silence_lose_them() {
    let mut net = Net::new(3);
    net.run(500, 50);
    for e in &net.e {
        assert_eq!(e.here(net.now).count(), 2, "{} sees the other two", e.name);
    }
    assert_eq!(net.e[0].peers[&101].name, "Ben");
    // Cam leaves quietly: gone once not heard for a while.
    net.away[2] = true;
    net.run(GONE_AFTER + HELLO_EVERY, 500);
    assert_eq!(net.e[0].here(net.now).count(), 1);
    assert!(net.e[0].events.iter().any(|ev| *ev == Event::Gone(102)));
    // Ben says goodbye: gone at once.
    let bye = net.e[1].bye().unwrap();
    net.deliver(1, vec![(To::All, bye)]);
    assert_eq!(net.e[0].here(net.now).count(), 0);
}

#[test]
fn each_message_is_kept_once_however_often_it_comes() {
    let mut net = Net::new(2);
    net.run(300, 50);
    let out = net.e[0].send(Body::Text("battery needed".into()), true, 1, net.now);
    let packet = out[0].1.clone();
    for _ in 0..3 {
        net.e[1].receive(&packet, None, net.now);
    }
    assert_eq!(text(&net.e[1]), ["battery needed"]);
    let arrived = net.e[1].events.iter().filter(|ev| matches!(ev, Event::Arrived { .. })).count();
    assert_eq!(arrived, 1, "said once on the island");
}

#[test]
fn every_handheld_shows_the_same_order() {
    let mut net = Net::new(3);
    net.run(300, 50);
    // Ana and Ben write at once, neither having heard the other: the same
    // Lamport time, told apart by who sent it.
    let a = net.e[0].send(Body::Text("from ana".into()), false, 1, net.now);
    let b = net.e[1].send(Body::Text("from ben".into()), false, 1, net.now);
    // Cam hears Ben's first, Ana and Ben each other's.
    net.deliver(1, b);
    net.deliver(0, a);
    net.send(2, Body::Text("from cam".into()));
    let want = ["from ana", "from ben", "from cam"];
    for e in &net.e {
        assert_eq!(text(e), want, "{}", e.name);
    }
    assert!(net.e[2].log[2].m.lamport > net.e[2].log[1].m.lamport, "after what it heard");
}

#[test]
fn a_lost_message_is_resent_until_acknowledged_then_left_alone() {
    let mut net = Net::new(2);
    net.run(300, 50);
    // Ana's first two sends of anything to Ben are lost.
    let mut lost = 0;
    net.lose = Box::new(move |from, _, p| {
        let msg = matches!(wire::open(&key(), p), Some((_, Frame::Msg { .. })));
        if from == 0 && msg && lost < 2 {
            lost += 1;
            return true;
        }
        false
    });
    net.send(0, Body::Call("robot to queue".into()));
    assert!(text(&net.e[1]).is_empty());
    assert_eq!(net.e[0].sending(), 1);
    net.run(3_000, 100);
    assert_eq!(text(&net.e[1]), ["robot to queue"], "the third send got there");
    assert_eq!(net.e[0].sending(), 0, "acknowledged: no more resends");
    assert!(net.e[0].log[0].got.contains(&101), "Ana knows Ben has it");
    // Nothing more goes out for it.
    let before = net.delivered;
    net.run(1_000, 100);
    assert!(net.delivered - before <= 2, "only hellos, if any");
}

#[test]
fn a_member_who_is_not_there_holds_up_no_resends() {
    let mut net = Net::new(2);
    net.run(300, 50);
    net.away[1] = true;
    net.run(GONE_AFTER + 1_000, 500);
    net.send(0, Body::Text("anyone?".into()));
    assert_eq!(net.e[0].sending(), 0, "no one here to wait for");
}

#[test]
fn whoever_was_away_is_handed_what_they_missed() {
    let mut net = Net::new(3);
    net.run(300, 50);
    net.send(0, Body::Text("before".into()));
    // Cam walks out of range; Ana and Ben talk on.
    net.away[2] = true;
    net.run(GONE_AFTER + 1_000, 500);
    for k in 0..5 {
        net.send(k % 2, Body::Text(format!("while away {k}")));
    }
    net.run(2_000, 100);
    // Cam comes back: its hello says what it has, and one of them hands on
    // the rest (only one: the lower-numbered).
    net.away[2] = false;
    let handed = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = handed.clone();
    net.lose = Box::new(move |_, to, p| {
        if to == 2 && matches!(wire::open(&key(), p), Some((_, Frame::Msg { late: true, .. }))) {
            counted.set(counted.get() + 1);
        }
        false
    });
    let hello = net.e[2].hello().unwrap();
    net.deliver(2, vec![(To::All, hello)]);
    net.run(1_000, 100);
    assert_eq!(text(&net.e[2]), text(&net.e[0]));
    assert_eq!(net.e[2].log.len(), 6);
    let late = net.e[2].events.iter().filter(|ev| matches!(ev, Event::Arrived { late: true, .. })).count();
    assert_eq!(late, 5, "the five missed, each once, as late");
    assert_eq!(handed.get(), 5, "handed on by one of them, not both");
}

#[test]
fn catching_up_reaches_back_two_hundred_and_passes_over_the_rest() {
    let mut net = Net::new(2);
    net.run(300, 50);
    net.away[1] = true;
    net.run(GONE_AFTER + 1_000, 500);
    for k in 0..(KEEP + 50) {
        net.send(0, Body::Text(format!("m{k}")));
    }
    assert_eq!(net.e[0].log.len(), KEEP, "the last 200 kept");
    net.away[1] = false;
    let hello = net.e[1].hello().unwrap();
    net.deliver(1, vec![(To::All, hello)]);
    net.run(500, 100);
    assert_eq!(net.e[1].log.len(), KEEP);
    assert_eq!(net.e[1].log[0].m.words(), "m50");
    // Its record of Ana's messages is whole: the first 50 passed over.
    assert_eq!(net.e[1].have[&100].upto, (KEEP + 50) as u32);
    assert!(net.e[1].have[&100].extra.is_empty());
}

#[test]
fn have_counts_each_once_and_floors_pass_over() {
    let mut h = Have::default();
    for s in [1, 3, 2, 5] {
        h.add(s);
    }
    assert_eq!((h.upto, h.extra.iter().copied().collect::<Vec<_>>()), (3, vec![5]));
    assert!(h.has(2) && h.has(5) && !h.has(4));
    h.floor(4);
    assert_eq!((h.upto, h.extra.len()), (5, 0));
}

#[test]
fn an_old_hello_played_back_changes_nothing() {
    let mut net = Net::new(2);
    let first = net.e[1].hello().unwrap();
    net.e[1].read_all(net.now);
    net.send(1, Body::Text("x".into()));
    net.e[1].read_all(net.now);
    let second = net.e[1].hello().unwrap();
    net.e[0].receive(&first, None, net.now);
    net.e[0].receive(&second, None, net.now);
    let seen = net.e[0].peers[&101].seen;
    assert_eq!(seen, 1);
    // Played back: not taken.
    net.e[0].receive(&first, None, net.now);
    assert_eq!(net.e[0].peers[&101].seen, seen);
    // A new boot (Ben restarted) is taken; the old boot's are not, after.
    let mut restarted = Engine::new(key(), 101, "Ben", 99);
    let third = restarted.hello().unwrap();
    net.e[0].receive(&third, None, net.now);
    assert_eq!(net.e[0].peers[&101].seen, 0, "the new boot's hello taken");
    net.e[0].receive(&second, None, net.now + 1);
    assert_eq!(net.e[0].peers[&101].seen, 0, "the old boot's not, after it");
}

#[test]
fn kept_and_taken_back_the_thread_is_the_same() {
    let mut net = Net::new(2);
    net.run(300, 50);
    net.send(0, Body::Call("all good".into()));
    net.send(1, Body::Share { kind: ShareKind::Scout, set: 4, part: 1, of: 1, label: "Scout Q31".into(), data: "SCOUT1 ...".into() });
    let kept = net.e[0].to_json();
    let mut again = Engine::new(key(), 100, "Ana", 5);
    again.restore(&kept);
    assert_eq!(text(&again), text(&net.e[0]));
    assert_eq!(again.next_seq, 2, "never numbers a message twice");
    assert_eq!(again.peers[&101].name, "Ben");
}

#[test]
fn three_members_with_a_lost_packet_agree() {
    // The multi-peer run: three handhelds, Ben's call lost on its way to
    // Cam the first time, and Cam's acknowledgement to Ben lost too.
    let mut net = Net::new(3);
    net.run(300, 50);
    let (mut lost_call, mut lost_ack) = (false, false);
    net.lose = Box::new(move |from, to, p| match wire::open(&key(), p) {
        Some((_, Frame::Msg { .. })) if from == 1 && to == 2 && !lost_call => {
            lost_call = true;
            true
        }
        Some((_, Frame::Ack(_))) if from == 2 && to == 1 && !lost_ack => {
            lost_ack = true;
            true
        }
        _ => false,
    });
    net.send(0, Body::Call("robot to queue".into()));
    net.send(1, Body::Call("battery needed".into()));
    net.send(2, Body::Text("on it".into()));
    net.run(5_000, 100);
    let want = text(&net.e[0]);
    assert_eq!(want.len(), 3);
    for e in &net.e {
        assert_eq!(text(e), want, "{}", e.name);
        assert_eq!(e.sending(), 0, "{}: everything acknowledged", e.name);
    }
}

#[test]
fn members_talk_over_real_sockets_on_this_computer() {
    // Three engines on localhost, each on its own port, "everyone" being
    // the other two: hellos, a message, its acknowledgements.
    let socks: Vec<UdpSocket> = (0..3).map(|_| UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap()).collect();
    let ports: Vec<u16> = socks.iter().map(|s| s.local_addr().unwrap().port()).collect();
    for s in &socks {
        s.set_read_timeout(Some(std::time::Duration::from_millis(5))).unwrap();
    }
    let mut e: Vec<Engine> = (0..3).map(|i| Engine::new(key(), 100 + i as u64, ["Ana", "Ben", "Cam"][i], 1 + i as u32)).collect();
    let send = |i: usize, out: Out, e: &[Engine], socks: &[UdpSocket]| {
        for (to, bytes) in out {
            for (t, port) in ports.iter().enumerate() {
                let aimed = match to {
                    To::All => t != i,
                    To::Peer(id) => e[t].me == id,
                };
                if aimed {
                    socks[i].send_to(&bytes, (Ipv4Addr::LOCALHOST, *port)).unwrap();
                }
            }
        }
    };
    let mut now = 5_000_000u64;
    let mut buf = [0u8; 2048];
    let mut said = false;
    for round in 0..60 {
        now += 50;
        for i in 0..3 {
            let out = e[i].poll(now);
            send(i, out, &e, &socks);
        }
        if round == 10 && !said {
            said = true;
            let out = e[0].send(Body::Call("bring the cart".into()), false, 1, now);
            send(0, out, &e, &socks);
        }
        for i in 0..3 {
            while let Ok((n, from)) = socks[i].recv_from(&mut buf) {
                let out = e[i].receive(&buf[..n], Some(from), now);
                send(i, out, &e, &socks);
            }
        }
    }
    for x in &e {
        assert_eq!(x.here(now).count(), 2, "{} found the others", x.name);
        assert_eq!(text(x), ["bring the cart"], "{}", x.name);
    }
    assert_eq!(e[0].sending(), 0);
    assert_eq!(e[0].log[0].got.len(), 3, "everyone has it");
}

// ------------------------------------------------------------------ join ---

#[test]
fn the_right_code_hands_over_the_key_and_a_wrong_one_nothing() {
    let k = key();
    let mut invite = Invite::new(0).unwrap();
    assert_eq!(invite.code.len(), 6);
    let (asking, ask) = Asking::new(&invite.code).unwrap();
    let answer = invite.answer(&ask, &k, "5805 pit", 1_000).unwrap();
    // Asked again (the joiner resending): the same answer, no try spent.
    assert_eq!(invite.answer(&ask, &k, "5805 pit", 1_100).unwrap(), answer);
    let (got, team) = asking.finish(&answer).unwrap();
    assert_eq!(got, k);
    assert_eq!(team, "5805 pit");
    // The key never crosses in the open.
    assert!(!answer.windows(8).any(|w| w == &k.bytes()[..8]));
    // A wrong code: the seal does not open.
    let wrong = if invite.code == "000000" { "000001" } else { "000000" };
    let (asking, ask) = Asking::new(wrong).unwrap();
    let answer = invite.answer(&ask, &k, "5805 pit", 1_200).unwrap();
    assert_eq!(asking.finish(&answer), None);
}

#[test]
fn a_code_answers_five_asks_and_five_minutes_only() {
    let k = key();
    let mut invite = Invite::new(0).unwrap();
    for _ in 0..super::join::TRIES {
        let (_, ask) = Asking::new("123456").unwrap();
        assert!(invite.answer(&ask, &k, "t", 10).is_some());
    }
    let (_, ask) = Asking::new(&invite.code).unwrap();
    assert!(invite.answer(&ask, &k, "t", 10).is_none(), "its tries are spent");
    let mut invite = Invite::new(0).unwrap();
    let (_, ask) = Asking::new(&invite.code).unwrap();
    assert!(invite.answer(&ask, &k, "t", super::join::LIFETIME).is_none(), "its time is up");
    assert!(invite.answer(b"CJ nonsense", &k, "t", 10).is_none());
}

// -------------------------------------------------------------- the QR ---

#[test]
fn a_team_qr_code_goes_there_and_back() {
    let m = super::Membership { key: key(), team: "5805 pit".into(), me: 3, name: "Thomas".into() };
    let text = m.qr_text();
    assert!(super::is_team_code(&text));
    let (k, team) = super::read_qr(&text).unwrap();
    assert_eq!((k, team.as_str()), (key(), "5805 pit"));
    assert!(!super::is_team_code("SCOUT1 1/1 abc"));
    assert!(!super::is_team_code("CATTEAM1 abcd 5805"), "a key cut short");
    assert_eq!(super::card(&text).1, "Team · 5805 pit");
}

#[test]
fn shares_pack_whole_and_come_back_in_order() {
    let texts: Vec<String> = (0..9).map(|k| format!("ST1:{k}:{}", "x".repeat(150))).collect();
    let parts = super::pack(&texts);
    assert!(parts.len() > 1 && parts.iter().all(|p| p.len() <= wire::DATA_MOST));
    // Sent by Ana, arriving out of order at Ben.
    let mut a = Engine::new(key(), 100, "Ana", 1);
    let mut b = Engine::new(key(), 101, "Ben", 2);
    let of = parts.len() as u8;
    let packets: Vec<Vec<u8>> = parts
        .iter()
        .enumerate()
        .map(|(k, data)| a.send(Body::Share { kind: ShareKind::Strategy, set: 9, part: k as u8 + 1, of, label: "board Q31".into(), data: data.clone() }, false, 1, 0).remove(0).1)
        .collect();
    for p in packets.iter().rev().skip(1) {
        b.receive(p, None, 0);
    }
    assert_eq!(super::share_texts(&b, 100, 9), None, "a part still to come");
    b.receive(&packets[parts.len() - 1], None, 0);
    assert_eq!(super::share_texts(&b, 100, 9), Some(texts));
}
