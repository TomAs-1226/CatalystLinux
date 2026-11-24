//! What the handheld's companion service (`catalyst-companiond`) and the
//! computer's companion app (`catalyst-companion.exe`) agree on.
//!
//! - **Finding it.** The app asks on UDP [`DISCOVERY_PORT`] (a broadcast on
//!   each network, and the USB cable's address); each handheld answers with
//!   a line of JSON: its id, name, version and the port it serves on.
//! - **Talking to it.** HTTPS on [`PORT`], TLS 1.3 only, with a self-signed
//!   certificate the handheld makes when it is first paired. The app pins
//!   that certificate's SHA-256 from then on: no other one is trusted.
//! - **Pairing** ([`pair`]). The handheld shows a six-digit code. The code
//!   never crosses the network: both ends run SPAKE2 with it, and each
//!   proves it has the same key over the certificate it saw, so someone in
//!   the middle (a different certificate) is found out, and someone
//!   listening learns nothing they could try codes against.
//! - **The token.** Pairing gives the app a long random token, which every
//!   request after carries (`Authorization: Bearer`). The handheld keeps
//!   only its SHA-256.
//! - **Backups** ([`tar`]) travel as a tar stream: the handheld writes and
//!   reads it with nothing more than this crate; the app keeps it as a zip.
//! - **The seal** ([`seal`]): XChaCha20-Poly1305 under derived keys, as
//!   Catalyst Team seals its packets.
//! - **Claude from the handheld** ([`claude`]): the handheld's Claude app
//!   and the bridge Companion runs on the computer, pairing by QR code or
//!   code and every message sealed end to end.

pub mod claude;
pub mod pair;
pub mod seal;
pub mod tar;

/// HTTPS: the API.
pub const PORT: u16 = 47315;
/// UDP: "is a handheld there?"
pub const DISCOVERY_PORT: u16 = 47316;
/// The question asked on [`DISCOVERY_PORT`]; anything else is not answered.
pub const DISCOVERY_ASK: &[u8] = b"catalyst-companion? 1";
/// The handheld over the USB cable, in its network mode.
pub const USB_ADDRESS: &str = "192.168.7.1";
/// Digits in a pairing code.
pub const CODE_DIGITS: usize = 6;
/// How long a pairing code is good for, in seconds.
pub const CODE_LIFETIME: u64 = 300;
/// Tries at one code before it is no good.
pub const TRIES: u32 = 5;

/// A code as typed: its digits alone (spaces and dashes are let go), or
/// `None` when it is not [`CODE_DIGITS`] digits.
pub fn clean_code(typed: &str) -> Option<String> {
    let digits: String = typed.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    (digits.len() == CODE_DIGITS && digits.chars().all(|c| c.is_ascii_digit())).then_some(digits)
}

/// Bytes as lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex back to bytes.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect()
}

/// A certificate's fingerprint: the SHA-256 of its DER.
pub fn fingerprint(der: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(der).into()
}

/// A token's SHA-256, as the handheld keeps it.
pub fn token_hash(token: &str) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(token.as_bytes()))
}

/// The same bytes, compared in a time that does not depend on where they differ.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_six_digits_however_typed() {
        assert_eq!(clean_code("482 913").as_deref(), Some("482913"));
        assert_eq!(clean_code(" 482-913 ").as_deref(), Some("482913"));
        assert_eq!(clean_code("48291"), None);
        assert_eq!(clean_code("48291a"), None);
        assert_eq!(clean_code("4829130"), None);
    }

    #[test]
    fn hex_goes_there_and_back() {
        let bytes = [0u8, 1, 0xab, 0xff];
        assert_eq!(hex(&bytes), "0001abff");
        assert_eq!(unhex("0001abff").as_deref(), Some(&bytes[..]));
        assert_eq!(unhex("0g"), None);
        assert_eq!(unhex("abc"), None);
        assert!(same(b"abc", b"abc") && !same(b"abc", b"abd") && !same(b"ab", b"abc"));
    }
}
