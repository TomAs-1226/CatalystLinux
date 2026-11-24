//! Team's key and its seal.
//!
//! - **The team key**: 32 random bytes from the kernel, made by the handheld
//!   that creates the team and handed on only by its QR code or, sealed, by
//!   a join with the six-digit code (join.rs). Everything between members
//!   is sealed with keys derived from it.
//! - **Derived, never used as it is**: SHA-256 of a label and the key. The
//!   seal's key ("catalyst-team seal") and the team's public id
//!   ("catalyst-team id", 8 bytes in every packet so another team's packets
//!   are let go without trying them) cannot be worked back to the key or to
//!   each other.
//! - **The seal** (companion-proto's `seal`, which Claude's messages use
//!   too): XChaCha20-Poly1305, as the Vault's: the A53 runs ChaCha at
//!   full speed without AES instructions, and its 24-byte nonce is drawn at
//!   random for every packet, so no counter has to survive a restart and two
//!   handhelds never need to agree on one (a repeat is a 2^-96 chance after
//!   2^48 packets). The packet's header is the associated data: a sender, a
//!   team or a count changed in transit does not open.

use zeroize::Zeroizing;

// The seal itself lives in companion-proto, shared with Claude's bridge
// protocol: one implementation for every sealed message the handheld sends.
pub use catalyst_companion_proto::seal::{derive, open_with, random, seal_with, KEY_LEN, OVERHEAD};
#[cfg(test)]
pub use catalyst_companion_proto::seal::NONCE_LEN;

/// A team's key, with what is derived from it.
#[derive(Clone)]
pub struct TeamKey {
    secret: Zeroizing<[u8; KEY_LEN]>,
    seal: Zeroizing<[u8; KEY_LEN]>,
    /// The team's public id, in every packet.
    pub id: [u8; 8],
}

impl std::fmt::Debug for TeamKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TeamKey({})", catalyst_companion_proto::hex(&self.id))
    }
}

impl PartialEq for TeamKey {
    fn eq(&self, other: &TeamKey) -> bool {
        catalyst_companion_proto::same(&self.secret[..], &other.secret[..])
    }
}

impl TeamKey {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> TeamKey {
        let secret = Zeroizing::new(bytes);
        let seal = derive(b"catalyst-team seal", &secret[..]);
        let mut id = [0u8; 8];
        id.copy_from_slice(&derive(b"catalyst-team id", &secret[..])[..8]);
        TeamKey { secret, seal, id }
    }

    /// A new team's key; none when the kernel gives no random numbers.
    pub fn random() -> Option<TeamKey> {
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        random(&mut bytes[..]).then(|| TeamKey::from_bytes(*bytes))
    }

    pub fn bytes(&self) -> &[u8; KEY_LEN] {
        &self.secret
    }

    /// `plain` sealed under the team's seal key with `aad`: a fresh nonce,
    /// then the sealed bytes and their tag.
    pub fn seal(&self, aad: &[u8], plain: &[u8]) -> Option<Vec<u8>> {
        seal_with(&self.seal, aad, plain)
    }

    /// What [`TeamKey::seal`] sealed, if it was this team's and nothing in
    /// it or its associated data changed.
    pub fn open(&self, aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
        open_with(&self.seal, aad, sealed)
    }
}
