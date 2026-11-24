//! Claude from the handheld: what the handheld's Claude app (the shell's
//! `claude/`) and the bridge on the computer (`catalyst-claude-bridge`,
//! which Catalyst Companion runs) agree on.
//!
//! - **Reaching it.** The bridge listens on 127.0.0.1:[`PORT`] only.
//!   Tailscale Funnel publishes that port at a fixed HTTPS address
//!   (`https://<computer>.<tailnet>.ts.net[:port]`), so the handheld needs
//!   nothing but HTTPS: no VPN, no Tailscale of its own.
//! - **Pairing, once.** The computer shows a QR code ([`Invite`]): the
//!   address, a random 256-bit key and the computer's name. The Camera
//!   reads it. Without the camera, a six-digit code ([`pairing`]): SPAKE2,
//!   as Companion pairs, over the same HTTPS; the code never crosses.
//! - **Sealed end to end.** Every request after carries its key's public id
//!   and is sealed ([`crate::seal`]) under a key derived from the pairing
//!   key for that direction, so the tunnel (and anyone between) reads
//!   nothing and changes nothing. The bridge refuses an id it does not know.
//! - **Each request once.** A request carries a random id and the time; the
//!   bridge takes an id once, and only within [`REPLAY_WINDOW_MS`] of its
//!   own clock (a handheld whose clock is off is told the time, sealed, and
//!   asks again). Its reply is sealed to that id, and a stream's events to
//!   the id and their place in the stream, so nothing can be swapped,
//!   replayed or reordered unseen.
//!
//! What crosses, inside the seal, is JSON: an `op` and its fields one way,
//! the answer (or an `error`) the other. The bridge sends text only: the
//! handheld never sees a file, a key or an account.

use serde_json::Value;
use zeroize::Zeroizing;

use crate::pair;
use crate::seal::{self, KEY_LEN};

/// TCP on 127.0.0.1: the bridge, as Funnel publishes it.
pub const PORT: u16 = 47318;
/// TCP on 127.0.0.1, never published: the permission prompt tool's way in.
pub const APPROVER_PORT: u16 = 47319;
/// UDP: the computer says where it is while it shows its pairing code, for
/// a handheld on the same network that types the code instead.
pub const ANNOUNCE_PORT: u16 = 47318;
/// An announcement's start; the computer's name and address follow.
pub const ANNOUNCE: &str = "catalyst-claude here 1\n";
/// A pairing QR code's start.
pub const QR_PREFIX: &str = "CATCLAUDE1 ";
/// Where sealed requests go.
pub const PATH: &str = "/v1/x";
/// Where a six-digit code is tried.
pub const PAIR_PATH: &str = "/v1/pair";
/// How far a request's time may be from the bridge's.
pub const REPLAY_WINDOW_MS: u64 = 5 * 60 * 1000;
/// How long the bridge waits for an answer to a permission request before
/// it denies it.
pub const APPROVAL_SECONDS: u64 = 5 * 60;

const REQUEST: &[u8] = b"catalyst-claude request\0";
const REPLY: &[u8] = b"catalyst-claude reply\0";
const EVENT: &[u8] = b"catalyst-claude event\0";

// ------------------------------------------------------------------ key ---

/// A pairing's key, with what is derived from it.
#[derive(Clone)]
pub struct Key {
    secret: Zeroizing<[u8; KEY_LEN]>,
    to_computer: Zeroizing<[u8; KEY_LEN]>,
    to_handheld: Zeroizing<[u8; KEY_LEN]>,
    /// Its public id, before every request, so the bridge knows which key
    /// to open it with (and refuses one it does not know).
    pub id: [u8; 8],
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "claude::Key({})", crate::hex(&self.id))
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Key) -> bool {
        crate::same(&self.secret[..], &other.secret[..])
    }
}

impl Key {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Key {
        let secret = Zeroizing::new(bytes);
        let to_computer = seal::derive(b"catalyst-claude to computer", &secret[..]);
        let to_handheld = seal::derive(b"catalyst-claude to handheld", &secret[..]);
        let mut id = [0u8; 8];
        id.copy_from_slice(&seal::derive(b"catalyst-claude id", &secret[..])[..8]);
        Key { secret, to_computer, to_handheld, id }
    }

    /// A new pairing's key; none when the system gives no random numbers.
    pub fn random() -> Option<Key> {
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        seal::random(&mut bytes[..]).then(|| Key::from_bytes(*bytes))
    }

    pub fn bytes(&self) -> &[u8; KEY_LEN] {
        &self.secret
    }

    pub fn id_hex(&self) -> String {
        crate::hex(&self.id)
    }

    /// A request: `op` with a fresh id and the time (`now_ms`, the
    /// handheld's clock as corrected by the bridge's), sealed. The id, for
    /// opening the reply, and the body to send.
    pub fn seal_request(&self, op: &Value, now_ms: u64) -> Option<(String, String)> {
        let mut rid = [0u8; 16];
        if !seal::random(&mut rid) {
            return None;
        }
        let rid = crate::hex(&rid);
        let mut v = op.clone();
        let obj = v.as_object_mut()?;
        obj.insert("rid".into(), Value::String(rid.clone()));
        obj.insert("t".into(), Value::from(now_ms));
        let plain = Zeroizing::new(v.to_string().into_bytes());
        let sealed = seal::seal_with(&self.to_computer, &request_aad(&self.id), &plain)?;
        Some((rid, format!("{}.{}", crate::hex(&self.id), b64::encode(&sealed))))
    }

    /// A request's JSON, if it was sealed with this key (its id already
    /// matched by [`request_id`]).
    pub fn open_request(&self, sealed_b64: &str) -> Option<Value> {
        let sealed = b64::decode(sealed_b64)?;
        let plain = Zeroizing::new(seal::open_with(&self.to_computer, &request_aad(&self.id), &sealed)?);
        let v: Value = serde_json::from_slice(&plain).ok()?;
        (v["rid"].as_str().is_some_and(|r| r.len() == 32) && v["t"].as_u64().is_some() && v["op"].is_string()).then_some(v)
    }

    /// The reply to request `rid`.
    pub fn seal_reply(&self, rid: &str, v: &Value) -> Option<String> {
        let sealed = seal::seal_with(&self.to_handheld, &[REPLY, rid.as_bytes()].concat(), v.to_string().as_bytes())?;
        Some(b64::encode(&sealed))
    }

    pub fn open_reply(&self, rid: &str, text: &str) -> Option<Value> {
        let plain = seal::open_with(&self.to_handheld, &[REPLY, rid.as_bytes()].concat(), &b64::decode(text.trim())?)?;
        serde_json::from_slice(&plain).ok()
    }

    /// The `index`th event (from 0) of the stream request `rid` opened.
    pub fn seal_event(&self, rid: &str, index: u64, v: &Value) -> Option<String> {
        let sealed = seal::seal_with(&self.to_handheld, &event_aad(rid, index), v.to_string().as_bytes())?;
        Some(b64::encode(&sealed))
    }

    pub fn open_event(&self, rid: &str, index: u64, text: &str) -> Option<Value> {
        let plain = seal::open_with(&self.to_handheld, &event_aad(rid, index), &b64::decode(text.trim())?)?;
        serde_json::from_slice(&plain).ok()
    }
}

fn request_aad(id: &[u8; 8]) -> Vec<u8> {
    [REQUEST, &id[..]].concat()
}

fn event_aad(rid: &str, index: u64) -> Vec<u8> {
    [EVENT, rid.as_bytes(), &index.to_be_bytes()].concat()
}

/// A request body's key id and its sealed part: `<id hex>.<sealed>`.
pub fn request_id(body: &str) -> Option<([u8; 8], &str)> {
    let (id, sealed) = body.trim().split_once('.')?;
    let id: [u8; 8] = crate::unhex(id)?.try_into().ok()?;
    Some((id, sealed))
}

// --------------------------------------------------------------- invite ---

/// What the computer's QR code holds: where it is, the key, and its name.
#[derive(Clone, Debug, PartialEq)]
pub struct Invite {
    pub address: String,
    pub key: Key,
    pub computer: String,
}

impl Invite {
    /// The QR code's text: `CATCLAUDE1 <address> <key hex> <name>`.
    pub fn qr_text(&self) -> String {
        let (head, name) = self.qr_parts();
        format!("{head}{name}")
    }

    /// The same in two parts for a compact code: all but the name in upper
    /// case (QR's alphanumeric mode, 5.5 bits a character rather than 8),
    /// and the name as it is.
    pub fn qr_parts(&self) -> (String, String) {
        (format!("{QR_PREFIX}{} {} ", self.address, crate::hex(self.key.bytes())).to_ascii_uppercase(), clean_name(&self.computer))
    }
}

/// A pairing QR code read.
pub fn read_qr(text: &str) -> Option<Invite> {
    let rest = text.trim().strip_prefix(QR_PREFIX)?;
    let mut parts = rest.splitn(3, ' ');
    let address = clean_address(parts.next()?)?;
    let key: [u8; KEY_LEN] = crate::unhex(parts.next()?)?.try_into().ok()?;
    let computer = clean_name(parts.next().unwrap_or("computer"));
    Some(Invite { address, key: Key::from_bytes(key), computer: if computer.is_empty() { "computer".into() } else { computer } })
}

/// An address as typed or read: `https://host[:port]` (https assumed),
/// with no path, query or anything a shell would read. None for anything else.
pub fn clean_address(typed: &str) -> Option<String> {
    // Upper case too: the QR code carries it so, in its compact alphanumeric mode.
    let t = typed.trim().trim_end_matches('/').to_ascii_lowercase();
    let rest = t.strip_prefix("https://").unwrap_or(&t);
    if rest.contains("://") || rest.is_empty() || rest.len() > 253 {
        return None;
    }
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, Some(p.parse::<u16>().ok().filter(|p| *p > 0)?)),
        None => (rest, None),
    };
    let ok = !host.is_empty() && host.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !l.starts_with('-'));
    if !ok {
        return None;
    }
    let host = host.to_ascii_lowercase();
    Some(match port {
        Some(443) | None => format!("https://{host}"),
        Some(p) => format!("https://{host}:{p}"),
    })
}

/// A computer's name as shown: printable, one line, at most 40 characters.
pub fn clean_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(40).collect::<String>().trim().to_owned()
}

// -------------------------------------------------------------- pairing ---

/// Pairing with a six-digit code, for a handheld that cannot read the QR
/// code: Companion's pairing (SPAKE2, [`crate::pair`]) with Claude's own
/// names for the two ends, so a code shown for one is never taken by the
/// other.
///
/// 1. The handheld sends SPAKE2's first message made from the code typed
///    ([`Asking::new`]) to [`PAIR_PATH`], never the code.
/// 2. The bridge answers it with its own message and the pairing key and
///    its name, sealed under a key derived from what SPAKE2 agreed
///    ([`answer`]). With a wrong code the two ends agree on different keys
///    and the seal does not open; each try costs one of the code's tries.
pub mod pairing {
    use super::*;

    const HANDHELD: &[u8] = b"catalyst-claude handheld";
    const COMPUTER: &[u8] = b"catalyst-claude computer";

    fn seal_key(agreed: &[u8]) -> Zeroizing<[u8; KEY_LEN]> {
        seal::derive(b"catalyst-claude pair", agreed)
    }

    fn aad(answer: &[u8]) -> Vec<u8> {
        [b"catalyst-claude pair\0".as_slice(), answer].concat()
    }

    /// The handheld's half, between its message and the bridge's answer.
    pub struct Asking(pair::Computer);

    impl Asking {
        pub fn new(code: &str) -> (Asking, Vec<u8>) {
            let (state, message) = pair::Computer::start_as(code, HANDHELD, COMPUTER);
            (Asking(state), message)
        }

        /// The key and the computer's name, if the code was the one shown.
        pub fn finish(self, answer: &[u8], sealed: &[u8]) -> Option<(Key, String)> {
            let agreed = Zeroizing::new(self.0.finish(answer).ok()?);
            let plain = Zeroizing::new(seal::open_with(&seal_key(&agreed), &aad(answer), sealed)?);
            if plain.len() < KEY_LEN {
                return None;
            }
            let key: [u8; KEY_LEN] = plain[..KEY_LEN].try_into().ok()?;
            Some((Key::from_bytes(key), clean_name(&String::from_utf8_lossy(&plain[KEY_LEN..]))))
        }
    }

    /// The bridge's answer to an ask made with `code`: its SPAKE2 message,
    /// and `key` and `computer` sealed.
    pub fn answer(code: &str, ask: &[u8], key: &Key, computer: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
        let (answer, agreed) = pair::answer_as(code, HANDHELD, COMPUTER, ask)?;
        let agreed = Zeroizing::new(agreed);
        let mut plain = Zeroizing::new(key.bytes().to_vec());
        plain.extend_from_slice(clean_name(computer).as_bytes());
        let sealed = seal::seal_with(&seal_key(&agreed), &aad(&answer), &plain).ok_or("no random numbers")?;
        Ok((answer, sealed))
    }
}

// --------------------------------------------------------------- base64 ---

/// URL-safe base64 without padding: what crosses as text.
pub mod b64 {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
            for i in 0..=chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
        }
        out
    }

    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let value = |c: u8| ALPHABET.iter().position(|&a| a == c).map(|p| p as u32);
        let bytes = text.as_bytes();
        if bytes.len() % 4 == 1 {
            return None;
        }
        let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
        for chunk in bytes.chunks(4) {
            let mut n = 0u32;
            for (i, &c) in chunk.iter().enumerate() {
                n |= value(c)? << (18 - 6 * i);
            }
            for i in 0..chunk.len() - 1 {
                out.push((n >> (16 - 8 * i)) as u8);
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base64_goes_there_and_back() {
        for n in 0..70usize {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let text = b64::encode(&bytes);
            assert!(text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
            assert_eq!(b64::decode(&text).as_deref(), Some(&bytes[..]), "{n} bytes");
        }
        assert_eq!(b64::encode(b"Man"), "TWFu");
        assert_eq!(b64::decode("TWE").as_deref(), Some(&b"Ma"[..]));
        assert_eq!(b64::decode("T"), None);
        assert_eq!(b64::decode("TW=="), None, "no padding");
    }

    #[test]
    fn a_sealed_request_and_its_reply_round_trip() {
        let key = Key::from_bytes([3; 32]);
        let (rid, body) = key.seal_request(&json!({"op": "sessions"}), 1_000).unwrap();
        let (id, sealed) = request_id(&body).unwrap();
        assert_eq!(id, key.id);
        assert!(!body.contains("sessions"), "nothing readable on the way");
        let v = key.open_request(sealed).unwrap();
        assert_eq!((v["op"].as_str(), v["rid"].as_str(), v["t"].as_u64()), (Some("sessions"), Some(rid.as_str()), Some(1_000)));
        // Another pairing's key opens nothing.
        assert!(Key::from_bytes([4; 32]).open_request(sealed).is_none());
        let reply = key.seal_reply(&rid, &json!({"ok": true})).unwrap();
        assert_eq!(key.open_reply(&rid, &reply), Some(json!({"ok": true})));
        assert_eq!(key.open_reply("0".repeat(32).as_str(), &reply), None, "bound to its request");
        // A request cannot pass for a reply, or the other way round.
        assert_eq!(key.open_reply(&rid, sealed), None);
        assert!(key.open_request(&reply).is_none());
        // Events: bound to the stream and their place in it.
        let e = key.seal_event(&rid, 2, &json!({"seq": 9})).unwrap();
        assert_eq!(key.open_event(&rid, 2, &e), Some(json!({"seq": 9})));
        assert_eq!(key.open_event(&rid, 3, &e), None, "out of place");
    }

    #[test]
    fn a_changed_request_does_not_open() {
        let key = Key::from_bytes([3; 32]);
        let (_, body) = key.seal_request(&json!({"op": "answer", "allow": false}), 5).unwrap();
        let (_, sealed) = request_id(&body).unwrap();
        let mut bytes = b64::decode(sealed).unwrap();
        let last = bytes.len() - 20;
        bytes[last] ^= 0x01;
        assert!(key.open_request(&b64::encode(&bytes)).is_none());
    }

    #[test]
    fn the_qr_code_reads_back() {
        let invite = Invite { address: "https://laptop.tail1234.ts.net:10000".into(), key: Key::from_bytes([9; 32]), computer: "THOMAS-LAPTOP".into() };
        let read = read_qr(&invite.qr_text()).unwrap();
        assert_eq!(read, invite);
        let (head, name) = invite.qr_parts();
        assert!(head.chars().all(|c| c.is_ascii_digit() || c.is_ascii_uppercase() || " $%*+-./:".contains(c)), "alphanumeric mode: {head}");
        assert_eq!(name, "THOMAS-LAPTOP");
        assert!(read_qr("CATTEAM1 abcd 5805").is_none(), "Team's code");
        assert!(read_qr("CATCLAUDE1 https://x.ts.net 1234 name").is_none(), "a short key");
        assert!(read_qr("CATCLAUDE1 https://x.ts.net/$(reboot) 0909 n").is_none());
    }

    #[test]
    fn addresses_are_https_hosts_only() {
        assert_eq!(clean_address("laptop.tail1234.ts.net").as_deref(), Some("https://laptop.tail1234.ts.net"));
        assert_eq!(clean_address(" https://Laptop.ts.net:10000/ ").as_deref(), Some("https://laptop.ts.net:10000"));
        assert_eq!(clean_address("https://laptop.ts.net:443").as_deref(), Some("https://laptop.ts.net"));
        for bad in ["http://laptop.ts.net", "laptop.ts.net/x", "laptop.ts.net:0", "laptop..net", "a b.net", "laptop.ts.net:99999", "", "-x.net", "x.net?q", "x.net;rm"] {
            assert_eq!(clean_address(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_code_pairs_and_a_wrong_one_does_not() {
        let key = Key::from_bytes([5; 32]);
        let (asking, ask) = pairing::Asking::new("482913");
        let (answer, sealed) = pairing::answer("482913", &ask, &key, "THOMAS-LAPTOP").unwrap();
        let (got, name) = asking.finish(&answer, &sealed).unwrap();
        assert_eq!((got, name.as_str()), (key.clone(), "THOMAS-LAPTOP"));
        let (asking, ask) = pairing::Asking::new("482914");
        let (answer, sealed) = pairing::answer("482913", &ask, &key, "x").unwrap();
        assert!(asking.finish(&answer, &sealed).is_none());
        // Companion's own pairing does not take Claude's message.
        let (_, ask) = pairing::Asking::new("482913");
        let (c_answer, _) = pair::handheld_answer("482913", &ask).unwrap();
        let (asking, _) = pairing::Asking::new("482913");
        assert!(asking.finish(&c_answer, &[0; 60]).is_none());
    }
}
