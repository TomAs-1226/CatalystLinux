//! The seal both ends use for what must not be read on the way: Catalyst
//! Team's packets (the shell's `team/`) and Claude's messages between the
//! handheld and the computer's bridge ([`crate::claude`]).
//!
//! - **Derived, never used as it is**: SHA-256 of a label, a zero and the
//!   secret. Keys derived under different labels cannot be worked back to
//!   the secret or to each other.
//! - **The seal**: XChaCha20-Poly1305. The A53 runs ChaCha at full speed
//!   without AES instructions, and its 24-byte nonce is drawn at random for
//!   every message, so no counter has to survive a restart and two ends
//!   never need to agree on one (a repeat is a 2^-96 chance after 2^48
//!   messages). The associated data is bound in: a header changed in
//!   transit does not open.

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{Key, Tag, XChaCha20Poly1305, XNonce};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
/// What a seal adds to what it seals.
pub const OVERHEAD: usize = NONCE_LEN + TAG_LEN;

/// SHA-256 of `label`, a zero and `secret`.
pub fn derive(label: &[u8], secret: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut h = Sha256::new();
    h.update(label);
    h.update([0u8]);
    h.update(secret);
    Zeroizing::new(h.finalize().into())
}

/// Bytes from the system's random source (getrandom(2) on the handheld).
pub fn random(buf: &mut [u8]) -> bool {
    getrandom::fill(buf).is_ok()
}

fn cipher(key: &[u8; KEY_LEN]) -> XChaCha20Poly1305 {
    let key: &Key = key[..].try_into().expect("a 32-byte key");
    XChaCha20Poly1305::new(key)
}

/// `plain` sealed under `key` with `aad`: a fresh nonce, then the sealed
/// bytes and their tag. None when the system gives no random numbers.
pub fn seal_with(key: &[u8; KEY_LEN], aad: &[u8], plain: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(OVERHEAD + plain.len());
    out.resize(NONCE_LEN, 0);
    if !random(&mut out[..NONCE_LEN]) {
        return None;
    }
    out.extend_from_slice(plain);
    let (nonce, body) = out.split_at_mut(NONCE_LEN);
    let nonce: &XNonce = (&*nonce).try_into().expect("a 24-byte nonce");
    let tag: Tag = cipher(key).encrypt_inout_detached(nonce, aad, body.into()).ok()?;
    out.extend_from_slice(&tag);
    Some(out)
}

/// Open what [`seal_with`] sealed, if it was sealed under this key and
/// nothing in it or its associated data changed.
pub fn open_with(key: &[u8; KEY_LEN], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < OVERHEAD {
        return None;
    }
    let (nonce, rest) = sealed.split_at(NONCE_LEN);
    let (body, tag) = rest.split_at(rest.len() - TAG_LEN);
    let nonce: &XNonce = nonce.try_into().ok()?;
    let tag: &Tag = tag.try_into().ok()?;
    let mut plain = body.to_vec();
    cipher(key).decrypt_inout_detached(nonce, aad, (&mut plain[..]).into(), tag).ok()?;
    Some(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seal_opens_only_with_its_key_and_its_header() {
        let key = [7u8; 32];
        let sealed = seal_with(&key, b"head", b"the words").unwrap();
        assert_eq!(sealed.len(), b"the words".len() + OVERHEAD);
        assert_eq!(open_with(&key, b"head", &sealed).as_deref(), Some(&b"the words"[..]));
        assert_eq!(open_with(&[8u8; 32], b"head", &sealed), None, "another key");
        assert_eq!(open_with(&key, b"hea", &sealed), None, "another header");
        let mut flipped = sealed.clone();
        flipped[NONCE_LEN + 2] ^= 1;
        assert_eq!(open_with(&key, b"head", &flipped), None, "a changed byte");
        assert_eq!(open_with(&key, b"head", &sealed[..OVERHEAD - 1]), None, "too short");
        assert_ne!(seal_with(&key, b"head", b"the words").unwrap(), sealed, "a fresh nonce each time");
    }

    #[test]
    fn labels_derive_unrelated_keys() {
        assert_ne!(*derive(b"a", b"secret"), *derive(b"b", b"secret"));
        assert_ne!(*derive(b"a", b"secret"), *derive(b"a", b"secreu"));
        assert_eq!(*derive(b"a", b"secret"), *derive(b"a", b"secret"));
    }
}
