//! Two-factor codes: TOTP (RFC 6238) over HOTP (RFC 4226), with SHA-1,
//! SHA-256 or SHA-512, 6 or 8 digits, 30 seconds a code unless the secret
//! says otherwise; and the `otpauth://` address a site's QR code holds.
//!
//! HMAC (RFC 2104) is written out here over the two hash crates the shell
//! already has, rather than a third crate for twenty lines.

use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    pub fn name(self) -> &'static str {
        match self {
            Algorithm::Sha1 => "SHA1",
            Algorithm::Sha256 => "SHA256",
            Algorithm::Sha512 => "SHA512",
        }
    }

    fn block(self) -> usize {
        match self {
            Algorithm::Sha512 => 128,
            _ => 64,
        }
    }

    fn hash(self, parts: &[&[u8]]) -> Zeroizing<Vec<u8>> {
        match self {
            Algorithm::Sha1 => {
                use sha1::Digest;
                let mut h = sha1::Sha1::new();
                for p in parts {
                    h.update(p);
                }
                Zeroizing::new(h.finalize().to_vec())
            }
            Algorithm::Sha256 => {
                use sha2::Digest;
                let mut h = sha2::Sha256::new();
                for p in parts {
                    h.update(p);
                }
                Zeroizing::new(h.finalize().to_vec())
            }
            Algorithm::Sha512 => {
                use sha2::Digest;
                let mut h = sha2::Sha512::new();
                for p in parts {
                    h.update(p);
                }
                Zeroizing::new(h.finalize().to_vec())
            }
        }
    }

    /// HMAC of `message` under `key`.
    pub fn hmac(self, key: &[u8], message: &[u8]) -> Zeroizing<Vec<u8>> {
        let block = self.block();
        let mut k = Zeroizing::new(vec![0u8; block]);
        if key.len() > block {
            let short = self.hash(&[key]);
            k[..short.len()].copy_from_slice(&short);
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut pad = Zeroizing::new(vec![0u8; block]);
        for (p, b) in pad.iter_mut().zip(k.iter()) {
            *p = b ^ 0x36;
        }
        let inner = self.hash(&[&pad, message]);
        for (p, b) in pad.iter_mut().zip(k.iter()) {
            *p = b ^ 0x5c;
        }
        self.hash(&[&pad, &inner])
    }
}

/// A two-factor secret and how its codes are made.
#[derive(Clone, PartialEq, Eq)]
pub struct Totp {
    secret: Zeroizing<Vec<u8>>,
    pub algorithm: Algorithm,
    pub digits: u32,
    pub period: u64,
    /// Who it is for, as the address says: the site, and the account.
    pub issuer: String,
    pub account: String,
}

impl std::fmt::Debug for Totp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Totp({}, {} digits, {} s, secret withheld)", self.algorithm.name(), self.digits, self.period)
    }
}

/// HOTP's code for `counter` (RFC 4226 § 5.3).
pub fn hotp(algorithm: Algorithm, secret: &[u8], counter: u64, digits: u32) -> u32 {
    let mac = algorithm.hmac(secret, &counter.to_be_bytes());
    let at = (mac[mac.len() - 1] & 0x0f) as usize;
    let bin = u32::from_be_bytes([mac[at] & 0x7f, mac[at + 1], mac[at + 2], mac[at + 3]]);
    bin % 10u32.pow(digits)
}

impl Totp {
    pub fn new(secret: &[u8], algorithm: Algorithm, digits: u32, period: u64) -> Totp {
        Totp { secret: Zeroizing::new(secret.to_vec()), algorithm, digits, period, issuer: String::new(), account: String::new() }
    }

    /// The code at `unix` seconds, as digits ("094287").
    pub fn code(&self, unix: i64) -> String {
        let step = (unix.max(0) as u64) / self.period.max(1);
        format!("{:0width$}", hotp(self.algorithm, &self.secret, step, self.digits), width = self.digits as usize)
    }

    /// Seconds the code at `unix` (with its fraction) has left, and how much
    /// of its period that is (1 just made, 0 about to change).
    pub fn left(&self, unix: f64) -> (u64, f32) {
        let period = self.period.max(1) as f64;
        let into = unix.rem_euclid(period);
        let left = period - into;
        (left.ceil() as u64, (left / period) as f32)
    }

    /// From what a person pastes or an export holds: an `otpauth://totp/`
    /// address, or the secret alone in base32 (with the defaults: SHA-1, 6
    /// digits, 30 seconds).
    pub fn parse(text: &str) -> Result<Totp, &'static str> {
        let text = text.trim();
        if text.is_empty() {
            return Err("no secret");
        }
        if text.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("otpauth:")) {
            return parse_uri(text);
        }
        let secret = base32(text).ok_or("not a base32 secret or otpauth:// address")?;
        if secret.len() < 10 {
            return Err("too short for a 2fa secret");
        }
        Ok(Totp::new(&secret, Algorithm::Sha1, 6, 30))
    }

    /// As an `otpauth://totp/` address: how the vault keeps it. Built in one
    /// buffer sized beforehand, wiped when let go, as it holds the secret.
    pub fn uri(&self) -> Zeroizing<String> {
        let label = match (self.issuer.is_empty(), self.account.is_empty()) {
            (false, false) => format!("{}:{}", escape(&self.issuer), escape(&self.account)),
            (false, true) => escape(&self.issuer),
            (true, false) => escape(&self.account),
            (true, true) => "code".to_owned(),
        };
        let issuer = (!self.issuer.is_empty()).then(|| format!("&issuer={}", escape(&self.issuer))).unwrap_or_default();
        let mut rest = issuer;
        if self.algorithm != Algorithm::Sha1 {
            rest.push_str(&format!("&algorithm={}", self.algorithm.name()));
        }
        if self.digits != 6 {
            rest.push_str(&format!("&digits={}", self.digits));
        }
        if self.period != 30 {
            rest.push_str(&format!("&period={}", self.period));
        }
        let secret = base32_encode(&self.secret);
        let mut out = Zeroizing::new(String::with_capacity(32 + label.len() + secret.len() + rest.len()));
        for part in ["otpauth://totp/", label.as_str(), "?secret=", secret.as_str(), rest.as_str()] {
            out.push_str(part);
        }
        out
    }
}

fn parse_uri(text: &str) -> Result<Totp, &'static str> {
    let rest = text.get(10..).filter(|_| text.get(..10).is_some_and(|s| s.eq_ignore_ascii_case("otpauth://"))).ok_or("not an otpauth:// address")?;
    let (kind, rest) = rest.split_once('/').ok_or("not an otpauth:// address")?;
    if kind.eq_ignore_ascii_case("hotp") {
        return Err("counter-based codes (hotp) are not supported");
    }
    if !kind.eq_ignore_ascii_case("totp") {
        return Err("not an otpauth://totp/ address");
    }
    let (label, query) = rest.split_once('?').unwrap_or((rest, ""));
    let label = unescape(label);
    let (mut issuer, account) = match label.split_once(':') {
        Some((i, a)) => (i.trim().to_owned(), a.trim().to_owned()),
        None => (String::new(), label.trim().to_owned()),
    };
    let mut secret = None;
    let (mut algorithm, mut digits, mut period) = (Algorithm::Sha1, 6, 30);
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = unescape(v);
        match k.to_ascii_lowercase().as_str() {
            "secret" => {
                let v = Zeroizing::new(v);
                secret = Some(base32(&v).ok_or("its secret is not base32")?);
            }
            "issuer" if !v.trim().is_empty() => issuer = v.trim().to_owned(),
            "algorithm" => {
                algorithm = match v.to_ascii_uppercase().replace('-', "").as_str() {
                    "SHA1" => Algorithm::Sha1,
                    "SHA256" => Algorithm::Sha256,
                    "SHA512" => Algorithm::Sha512,
                    _ => return Err("its algorithm is not sha-1, sha-256 or sha-512"),
                }
            }
            "digits" => digits = v.parse().ok().filter(|d| (6..=8).contains(d)).ok_or("codes of 6, 7 or 8 digits only")?,
            "period" => period = v.parse().ok().filter(|p| (5..=600).contains(p)).ok_or("its period is not a few seconds to ten minutes")?,
            _ => {}
        }
    }
    let secret = Zeroizing::new(secret.ok_or("no secret in the address")?);
    if secret.len() < 10 {
        return Err("too short for a 2fa secret");
    }
    let mut t = Totp::new(&secret, algorithm, digits, period);
    t.issuer = issuer;
    t.account = account;
    Ok(t)
}

/// RFC 4648 base32, any case, spaces, dashes and padding passed over.
pub fn base32(text: &str) -> Option<Zeroizing<Vec<u8>>> {
    let mut out = Zeroizing::new(Vec::with_capacity(text.len() * 5 / 8 + 1));
    let (mut bits, mut acc) = (0u32, 0u64);
    for c in text.chars() {
        let v = match c.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u64 - 'A' as u64,
            c @ '2'..='7' => c as u64 - '2' as u64 + 26,
            ' ' | '-' | '=' => continue,
            _ => return None,
        };
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    acc.zeroize();
    (!out.is_empty()).then_some(out)
}

fn base32_encode(bytes: &[u8]) -> Zeroizing<String> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = Zeroizing::new(String::with_capacity(bytes.len() * 8 / 5 + 2));
    let (mut bits, mut acc) = (0u32, 0u64);
    for b in bytes {
        acc = (acc << 8) | *b as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    acc.zeroize();
    out
}

fn unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 3 <= bytes.len() => match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => {
                    out.push(v);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotp_gives_rfc_4226s_codes() {
        // RFC 4226, Appendix D: the secret "12345678901234567890", counters 0 to 9.
        let want = [755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489];
        for (counter, code) in want.into_iter().enumerate() {
            assert_eq!(hotp(Algorithm::Sha1, b"12345678901234567890", counter as u64, 6), code, "counter {counter}");
        }
    }

    #[test]
    fn totp_gives_rfc_6238s_codes_for_each_hash() {
        // RFC 6238, Appendix B: 8 digits, 30 s, each hash with its own seed.
        let sha1 = Totp::new(b"12345678901234567890", Algorithm::Sha1, 8, 30);
        let sha256 = Totp::new(b"12345678901234567890123456789012", Algorithm::Sha256, 8, 30);
        let sha512 = Totp::new(b"1234567890123456789012345678901234567890123456789012345678901234", Algorithm::Sha512, 8, 30);
        let table: [(i64, &str, &str, &str); 6] = [
            (59, "94287082", "46119246", "90693936"),
            (1111111109, "07081804", "68084774", "25091201"),
            (1111111111, "14050471", "67062674", "99943326"),
            (1234567890, "89005924", "91819424", "93441116"),
            (2000000000, "69279037", "90698825", "38618901"),
            (20000000000, "65353130", "77737706", "47863826"),
        ];
        for (t, a, b, c) in table {
            assert_eq!((sha1.code(t).as_str(), sha256.code(t).as_str(), sha512.code(t).as_str()), (a, b, c), "at {t}");
        }
        // Six digits: the last six of the same.
        assert_eq!(Totp::new(b"12345678901234567890", Algorithm::Sha1, 6, 30).code(59), "287082");
    }

    #[test]
    fn hmac_matches_rfc_4231s_vectors_and_rfc_2202s() {
        // RFC 2202 test case 2 (SHA-1) and RFC 4231 test case 2 (SHA-256, SHA-512).
        let hex = |m: &[u8]| m.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(&Algorithm::Sha1.hmac(b"Jefe", b"what do ya want for nothing?")), "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79");
        assert_eq!(hex(&Algorithm::Sha256.hmac(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert!(hex(&Algorithm::Sha512.hmac(b"Jefe", b"what do ya want for nothing?")).starts_with("164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea250554"));
        // A key longer than a block is hashed first (RFC 4231 test case 6).
        let long = [0xaau8; 131];
        assert_eq!(hex(&Algorithm::Sha256.hmac(&long, b"Test Using Larger Than Block-Size Key - Hash Key First")), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn the_countdown_runs_from_the_whole_period_to_nothing() {
        let t = Totp::new(b"12345678901234567890", Algorithm::Sha1, 6, 30);
        assert_eq!(t.left(60.0), (30, 1.0));
        let (s, f) = t.left(89.5);
        assert_eq!(s, 1);
        assert!((f - 0.5 / 30.0).abs() < 1e-4);
        assert_eq!(t.code(60), t.code(89));
        assert_ne!(t.code(89), t.code(90));
    }

    #[test]
    fn otpauth_addresses_and_bare_secrets_are_read() {
        let t = Totp::parse("otpauth://totp/ACME%20Co:jane.doe@example.org?secret=JBSWY3DPEHPK3PXP&issuer=ACME%20Co&algorithm=SHA256&digits=8&period=60").unwrap();
        assert_eq!((t.issuer.as_str(), t.account.as_str(), t.algorithm, t.digits, t.period), ("ACME Co", "jane.doe@example.org", Algorithm::Sha256, 8, 60));
        assert_eq!(&t.secret[..], b"Hello!\xde\xad\xbe\xef");
        // Kept as an address, it reads back the same.
        assert_eq!(Totp::parse(&t.uri()).unwrap(), t);
        let bare = Totp::parse("jbsw y3dp ehpk 3pxp").unwrap();
        assert_eq!((bare.algorithm, bare.digits, bare.period), (Algorithm::Sha1, 6, 30));
        assert_eq!(Totp::parse("otpauth://totp/x?secret=JBSWY3DPEHPK3PXP").unwrap().uri().as_str(), "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP");
        for bad in ["", "hello world!", "otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP&counter=1", "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=9", "otpauth://totp/x?issuer=y", "otpauth://totp/x?secret=AB", "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&algorithm=MD5"] {
            assert!(Totp::parse(bad).is_err(), "{bad:?}");
        }
        assert!(!format!("{t:?}").contains("JBSW"));
    }
}
