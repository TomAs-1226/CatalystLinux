//! The vault's file and its keys: what is on the card, and how it is opened.
//!
//! - **Two keys.** The entries are sealed with the *vault key*, 32 random
//!   bytes made once, with the vault. The vault key is itself sealed
//!   ("wrapped") with a key derived from the passcode. Changing the passcode
//!   wraps the same vault key again under the new one; the entries need not
//!   be read to do it, and nothing is ever sealed under two passcodes.
//! - **The derivation.** Argon2id (RFC 9106) of the passcode with a random
//!   16-byte salt of the vault's own (never the lock screen's: the lock
//!   screen's hash is on the card for anyone to read), at a cost written in
//!   the file. Its 19 MiB of working memory is wiped when it is done.
//! - **The seal.** XChaCha20-Poly1305 for both: ChaCha's adds, rotations and
//!   xors are what the A53 does at full speed without AES instructions, and
//!   its 24-byte nonce is drawn at random for every write, with no counter
//!   to keep. Everything is authenticated: the header, as the wrap's
//!   associated data; the header with the wrapped key and the body's nonce,
//!   as the body's. A byte changed anywhere, a header from another file, a
//!   body cut short: none of them opens.
//! - **The body** is padded to a whole number of 256-byte blocks, so the file
//!   does not tell how long each password is.
//!
//! The file, all little-endian:
//!
//! | at | bytes | what |
//! | --- | --- | --- |
//! | 0 | 8 | `CATVAULT` |
//! | 8 | 1 | the format's version, 1 |
//! | 9 | 1 | what it is: 0 the vault, 1 a backup |
//! | 10 | 1 | the derivation: 1, Argon2id v1.3 |
//! | 11 | 1 | 0 |
//! | 12 | 12 | its cost: memory in KiB, passes, lanes |
//! | 24 | 16 | the salt |
//! | 40 | 8 | which lock-screen passcode it was wrapped under (below) |
//! | 48 | 24 | the wrap's nonce |
//! | 72 | 48 | the vault key, sealed (32 and the 16-byte tag); bytes 0–72 its associated data |
//! | 120 | 24 | the body's nonce, new at every write |
//! | 144 | … | the body, sealed (a multiple of 256, and the tag); bytes 0–144 its associated data |
//!
//! The passcode's mark (at 40) is the first 8 bytes of SHA-256 of the lock
//! screen's hash string, which is on the card already: it says nothing of
//! the passcode, only whether the one set now is the one the vault was
//! wrapped under (it may not be, if the passcode came off over SSH).

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{Key, Tag, XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, Zeroizing};

use crate::passcode::Cost;

pub const MAGIC: &[u8; 8] = b"CATVAULT";
pub const VERSION: u8 = 1;
const KDF_ARGON2ID: u8 = 1;

const AT_VERSION: usize = 8;
const AT_ROLE: usize = 9;
const AT_KDF: usize = 10;
const AT_COST: usize = 12;
const AT_SALT: usize = 24;
const AT_MARK: usize = 40;
const AT_WRAP_NONCE: usize = 48;
const AT_WRAPPED: usize = 72;
const AT_BODY_NONCE: usize = 120;
/// The header: everything before the body.
pub const HEAD: usize = 144;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
/// The body is padded to a multiple of this.
pub const BLOCK: usize = 256;

/// What a file is for: the vault itself, or a backup of it. Written in the
/// header and authenticated, so one cannot be passed off as the other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Vault = 0,
    Backup = 1,
}

/// Why a file would not open. None of them says anything of what is in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not a vault (or a backup where a vault was expected).
    NotOurs,
    /// Made by a newer Catalyst Linux.
    Newer,
    /// Too short, or its header makes no sense.
    Damaged,
    /// The passcode (or the backup's password) is not the one, or the
    /// header was changed: the two cannot be told apart.
    Wrong,
    /// The key opened but the body did not: changed, or damaged.
    Tampered,
    /// The kernel gave no random numbers.
    Random,
    /// The derivation would not run at that cost (out of memory).
    Cost,
}

impl Error {
    /// In plain words, for the island.
    pub fn words(self) -> &'static str {
        match self {
            Error::NotOurs => "not a vault file",
            Error::Newer => "made by a newer catalyst · update first",
            Error::Damaged => "the file is damaged",
            Error::Wrong => "not that one",
            Error::Tampered => "the file was changed or damaged · it will not open",
            Error::Random => "no random numbers from the system",
            Error::Cost => "not enough memory to open it just now",
        }
    }
}

/// Bytes from the kernel's random source.
pub fn random(buf: &mut [u8]) -> Result<(), Error> {
    getrandom::fill(buf).map_err(|_| Error::Random)
}

/// A cost a file may name: within reason, so a damaged or hostile file
/// cannot ask for more memory than a backup's own 64 MiB, nor for minutes
/// (eight passes at that size are a few seconds on the A53).
pub fn sane(cost: Cost) -> bool {
    (8..=64 * 1024).contains(&cost.m_kib) && (1..=8).contains(&cost.t) && cost.p == 1
}

/// The key `secret` derives with `salt` at `cost`: Argon2id, its working
/// memory wiped before it is let go.
pub fn derive(secret: &[u8], salt: &[u8], cost: Cost) -> Result<Zeroizing<[u8; KEY_LEN]>, Error> {
    use argon2::{Algorithm, Argon2, Block, Params, Version};
    if !sane(cost) {
        return Err(Error::Damaged);
    }
    let params = Params::new(cost.m_kib, cost.t, cost.p, Some(KEY_LEN)).map_err(|_| Error::Cost)?;
    let mut blocks = Zeroizing::new(vec![Block::default(); params.block_count()]);
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon.hash_password_into_with_memory(secret, salt, &mut out[..], &mut blocks[..]).map_err(|_| Error::Cost)?;
    Ok(out)
}

/// Which lock-screen passcode a vault is wrapped under: the first 8 bytes of
/// SHA-256 of its hash string (zeros for none).
pub fn mark_of(phc: Option<&str>) -> [u8; 8] {
    use sha2::{Digest, Sha256};
    let Some(phc) = phc else { return [0; 8] };
    let sum = Sha256::digest(phc.trim().as_bytes());
    let mut out = [0u8; 8];
    out.copy_from_slice(&sum[..8]);
    out
}

fn cipher(key: &[u8]) -> XChaCha20Poly1305 {
    let key: &Key = key.try_into().expect("a 32-byte key");
    XChaCha20Poly1305::new(key)
}

fn nonce(bytes: &[u8]) -> &XNonce {
    bytes.try_into().expect("a 24-byte nonce")
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

/// What a file's header says, read without any secret.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Head {
    pub role: Role,
    pub cost: Cost,
    pub mark: [u8; 8],
}

impl Head {
    /// The header of `file`, if it is one of ours whole enough to try.
    pub fn read(file: &[u8]) -> Result<Head, Error> {
        if file.len() < MAGIC.len() || &file[..MAGIC.len()] != MAGIC {
            return Err(Error::NotOurs);
        }
        if file.len() < HEAD {
            return Err(Error::Damaged);
        }
        match file[AT_VERSION] {
            0 => return Err(Error::Damaged),
            v if v > VERSION => return Err(Error::Newer),
            _ => {}
        }
        if file[AT_KDF] != KDF_ARGON2ID {
            return Err(if file[AT_KDF] > KDF_ARGON2ID { Error::Newer } else { Error::Damaged });
        }
        let role = match file[AT_ROLE] {
            0 => Role::Vault,
            1 => Role::Backup,
            _ => return Err(Error::Damaged),
        };
        let cost = Cost { m_kib: u32_at(file, AT_COST), t: u32_at(file, AT_COST + 4), p: u32_at(file, AT_COST + 8) };
        let body = file.len() - HEAD;
        if !sane(cost) || body < BLOCK + TAG_LEN || (body - TAG_LEN) % BLOCK != 0 {
            return Err(Error::Damaged);
        }
        let mut mark = [0u8; 8];
        mark.copy_from_slice(&file[AT_MARK..AT_MARK + 8]);
        Ok(Head { role, cost, mark })
    }
}

/// An opened body: the decrypted bytes, wiped when it is let go.
pub struct Plain {
    buf: Zeroizing<Vec<u8>>,
    len: usize,
}

impl Plain {
    pub fn bytes(&self) -> &[u8] {
        &self.buf[HEAD + 4..HEAD + 4 + self.len]
    }
}

/// The vault key, and the header it is wrapped in: what an open vault holds.
/// The key is wiped when this is let go; the header is not secret.
pub struct Keyring {
    key: Zeroizing<[u8; KEY_LEN]>,
    head: [u8; AT_BODY_NONCE],
}

impl Keyring {
    /// A new vault key, wrapped under `secret`.
    pub fn make(secret: &[u8], cost: Cost, role: Role, mark: [u8; 8]) -> Result<Keyring, Error> {
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        random(&mut key[..])?;
        Self::wrap(key, secret, cost, role, mark)
    }

    fn wrap(key: Zeroizing<[u8; KEY_LEN]>, secret: &[u8], cost: Cost, role: Role, mark: [u8; 8]) -> Result<Keyring, Error> {
        let mut head = [0u8; AT_BODY_NONCE];
        head[..MAGIC.len()].copy_from_slice(MAGIC);
        head[AT_VERSION] = VERSION;
        head[AT_ROLE] = role as u8;
        head[AT_KDF] = KDF_ARGON2ID;
        for (i, v) in [cost.m_kib, cost.t, cost.p].into_iter().enumerate() {
            head[AT_COST + 4 * i..AT_COST + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        random(&mut head[AT_SALT..AT_SALT + 16])?;
        head[AT_MARK..AT_MARK + 8].copy_from_slice(&mark);
        random(&mut head[AT_WRAP_NONCE..AT_WRAPPED])?;
        let kek = derive(secret, &head[AT_SALT..AT_SALT + 16], cost)?;
        let (aad, rest) = head.split_at_mut(AT_WRAPPED);
        let (wrapped, tag_out) = rest.split_at_mut(KEY_LEN);
        wrapped.copy_from_slice(&key[..]);
        let tag = cipher(&kek[..]).encrypt_inout_detached(nonce(&aad[AT_WRAP_NONCE..AT_WRAPPED]), aad, wrapped.into());
        match tag {
            Ok(tag) => tag_out.copy_from_slice(&tag),
            Err(_) => {
                wrapped.zeroize();
                return Err(Error::Damaged);
            }
        }
        Ok(Keyring { key, head })
    }

    /// The same vault key, wrapped under another secret (a new passcode),
    /// with a new salt and nonce.
    pub fn rewrap(&self, secret: &[u8], cost: Cost, mark: [u8; 8]) -> Result<Keyring, Error> {
        Self::wrap(self.key.clone(), secret, cost, self.head().role, mark)
    }

    /// The same vault key in a header of another role (a backup's seal is
    /// made with `make`; this is for the tests' swapped headers).
    #[cfg(test)]
    fn head_bytes(&self) -> [u8; AT_BODY_NONCE] {
        self.head
    }

    pub fn head(&self) -> Head {
        let mut file = Vec::with_capacity(HEAD + BLOCK + TAG_LEN);
        file.extend_from_slice(&self.head);
        file.resize(HEAD + BLOCK + TAG_LEN, 0);
        Head::read(&file).expect("its own header reads")
    }

    /// The whole file for `payload`: the header, a new nonce, the body
    /// padded and sealed. The plaintext is only ever in the buffer that
    /// becomes the file, and is sealed where it lies.
    pub fn seal(&self, payload: &[u8]) -> Result<Vec<u8>, Error> {
        let inner = 4 + payload.len();
        let padded = inner.div_ceil(BLOCK).max(1) * BLOCK;
        let total = HEAD + padded + TAG_LEN;
        let mut out = Zeroizing::new(Vec::with_capacity(total));
        out.extend_from_slice(&self.head);
        out.resize(HEAD, 0);
        random(&mut out[AT_BODY_NONCE..HEAD])?;
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out.resize(HEAD + padded, 0);
        let (aad, body) = out.split_at_mut(HEAD);
        let tag: Tag = cipher(&self.key[..]).encrypt_inout_detached(nonce(&aad[AT_BODY_NONCE..HEAD]), aad, body.into()).map_err(|_| Error::Damaged)?;
        out.extend_from_slice(&tag);
        debug_assert_eq!(out.capacity(), total, "never grown, so never copied");
        // Sealed now: nothing secret is left in it.
        Ok(std::mem::take(&mut *out))
    }

    /// Open `file`'s body with this vault key.
    pub fn unseal(&self, file: &[u8]) -> Result<Plain, Error> {
        Head::read(file)?;
        let mut buf = Zeroizing::new(file.to_vec());
        let (aad, rest) = buf.split_at_mut(HEAD);
        let (body, tag) = rest.split_at_mut(rest.len() - TAG_LEN);
        let tag: &Tag = (&*tag).try_into().expect("a 16-byte tag");
        cipher(&self.key[..]).decrypt_inout_detached(nonce(&aad[AT_BODY_NONCE..HEAD]), aad, body.into(), tag).map_err(|_| Error::Tampered)?;
        let len = u32_at(&buf, HEAD) as usize;
        if 4 + len > buf.len() - HEAD - TAG_LEN {
            return Err(Error::Damaged);
        }
        Ok(Plain { buf, len })
    }

    /// Open `file` with `secret`: its vault key unwrapped, its body opened.
    pub fn open(file: &[u8], secret: &[u8], role: Role) -> Result<(Keyring, Plain), Error> {
        let head = Head::read(file)?;
        if head.role != role {
            return Err(Error::NotOurs);
        }
        let kek = derive(secret, &file[AT_SALT..AT_SALT + 16], head.cost)?;
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        key.copy_from_slice(&file[AT_WRAPPED..AT_WRAPPED + KEY_LEN]);
        let tag: &Tag = file[AT_WRAPPED + KEY_LEN..AT_BODY_NONCE].try_into().expect("a 16-byte tag");
        cipher(&kek[..]).decrypt_inout_detached(nonce(&file[AT_WRAP_NONCE..AT_WRAPPED]), &file[..AT_WRAPPED], (&mut key[..]).into(), tag).map_err(|_| Error::Wrong)?;
        let mut header = [0u8; AT_BODY_NONCE];
        header.copy_from_slice(&file[..AT_BODY_NONCE]);
        let ring = Keyring { key, head: header };
        let plain = ring.unseal(file)?;
        Ok((ring, plain))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUICK: Cost = Cost { m_kib: 64, t: 1, p: 1 };

    #[test]
    fn a_vault_seals_and_opens_with_its_passcode_alone() {
        let ring = Keyring::make(b"2468", QUICK, Role::Vault, [7; 8]).unwrap();
        let secret = b"hunter2 is my password";
        let file = ring.seal(secret).unwrap();
        // Nothing of it in the file, which is padded to whole blocks.
        assert!(!file.windows(7).any(|w| w == b"hunter2"));
        assert_eq!((file.len() - HEAD - TAG_LEN) % BLOCK, 0);
        assert_eq!(Head::read(&file).unwrap(), Head { role: Role::Vault, cost: QUICK, mark: [7; 8] });
        let (again, plain) = Keyring::open(&file, b"2468", Role::Vault).unwrap();
        assert_eq!(plain.bytes(), secret);
        // Opened, it seals again under the same key, a new nonce each time.
        let second = again.seal(b"another").unwrap();
        assert_ne!(file[AT_BODY_NONCE..HEAD], second[AT_BODY_NONCE..HEAD]);
        assert_eq!(file[..AT_BODY_NONCE], second[..AT_BODY_NONCE], "the wrapped key is kept as it was");
        assert_eq!(Keyring::open(&second, b"2468", Role::Vault).unwrap().1.bytes(), b"another");
        // An empty vault, and one larger than a block.
        let big = vec![b'x'; 3 * BLOCK];
        for payload in [&b""[..], &big[..]] {
            let f = ring.seal(payload).unwrap();
            assert_eq!(Keyring::open(&f, b"2468", Role::Vault).unwrap().1.bytes(), payload);
        }
    }

    #[test]
    fn a_wrong_passcode_does_not_open_it() {
        let ring = Keyring::make(b"2468", QUICK, Role::Vault, [0; 8]).unwrap();
        let file = ring.seal(b"secret").unwrap();
        for wrong in [&b"2469"[..], b"246", b"24680", b"", b" 2468"] {
            assert_eq!(Keyring::open(&file, wrong, Role::Vault).err(), Some(Error::Wrong), "{wrong:?}");
        }
    }

    #[test]
    fn every_byte_changed_is_caught() {
        let ring = Keyring::make(b"correct horse", QUICK, Role::Vault, [3; 8]).unwrap();
        let file = ring.seal(b"the elevator's wifi is frc5805!").unwrap();
        for at in 0..file.len() {
            let mut bad = file.clone();
            bad[at] ^= 0x01;
            let opened = Keyring::open(&bad, b"correct horse", Role::Vault);
            assert!(opened.is_err(), "byte {at} changed and it still opened");
        }
        // Cut short, grown, or a block dropped: none opens.
        for bad in [&file[..file.len() - 1], &file[..HEAD + 10], &file[..file.len() - BLOCK]] {
            assert!(Keyring::open(bad, b"correct horse", Role::Vault).is_err());
        }
        let mut longer = file.clone();
        longer.extend_from_slice(&[0; BLOCK]);
        assert!(Keyring::open(&longer, b"correct horse", Role::Vault).is_err());
    }

    #[test]
    fn a_header_from_another_vault_or_a_backup_does_not_open_a_body() {
        let a = Keyring::make(b"2468", QUICK, Role::Vault, [0; 8]).unwrap();
        let b = Keyring::make(b"2468", QUICK, Role::Vault, [0; 8]).unwrap();
        let fa = a.seal(b"vault a").unwrap();
        let fb = b.seal(b"vault b").unwrap();
        // B's header and wrapped key over A's body: B's key opens, the body does not.
        let mut swapped = fa.clone();
        swapped[..AT_BODY_NONCE].copy_from_slice(&b.head_bytes());
        assert_eq!(Keyring::open(&swapped, b"2468", Role::Vault).err(), Some(Error::Tampered));
        assert!(Keyring::open(&fb, b"2468", Role::Vault).is_ok());
        // A backup is not a vault, and the other way round.
        let backup = Keyring::make(b"a long backup password", QUICK, Role::Backup, [0; 8]).unwrap().seal(b"x").unwrap();
        assert_eq!(Keyring::open(&backup, b"a long backup password", Role::Vault).err(), Some(Error::NotOurs));
        assert!(Keyring::open(&backup, b"a long backup password", Role::Backup).is_ok());
        let mut relabelled = backup.clone();
        relabelled[AT_ROLE] = Role::Vault as u8;
        assert_eq!(Keyring::open(&relabelled, b"a long backup password", Role::Vault).err(), Some(Error::Wrong), "the role is authenticated");
    }

    #[test]
    fn a_new_passcode_wraps_the_same_key_and_the_old_one_no_longer_opens() {
        let old = Keyring::make(b"2468", QUICK, Role::Vault, [1; 8]).unwrap();
        let file = old.seal(b"entries").unwrap();
        let new = old.rewrap(b"new passcode", QUICK, [2; 8]).unwrap();
        // The body sealed again under the new header (its associated data).
        let plain = old.unseal(&file).unwrap();
        let moved = new.seal(plain.bytes()).unwrap();
        assert_eq!(Head::read(&moved).unwrap().mark, [2; 8]);
        assert_ne!(moved[AT_SALT..AT_SALT + 16], file[AT_SALT..AT_SALT + 16], "a new salt");
        assert_eq!(Keyring::open(&moved, b"2468", Role::Vault).err(), Some(Error::Wrong));
        let (ring, plain) = Keyring::open(&moved, b"new passcode", Role::Vault).unwrap();
        assert_eq!(plain.bytes(), b"entries");
        // The same vault key: what it seals the old ring opens.
        assert!(old.unseal(&ring.seal(b"x").unwrap()).is_ok());
    }

    #[test]
    fn only_our_files_at_a_sane_cost_are_tried() {
        let ring = Keyring::make(b"2468", QUICK, Role::Vault, [0; 8]).unwrap();
        let file = ring.seal(b"x").unwrap();
        assert_eq!(Head::read(b"username,password\n").err(), Some(Error::NotOurs));
        assert_eq!(Head::read(&file[..60]).err(), Some(Error::Damaged));
        let mut newer = file.clone();
        newer[AT_VERSION] = VERSION + 1;
        assert_eq!(Head::read(&newer).err(), Some(Error::Newer));
        let mut huge = file.clone();
        huge[AT_COST..AT_COST + 4].copy_from_slice(&(4u32 << 20).to_le_bytes());
        assert_eq!(Head::read(&huge).err(), Some(Error::Damaged), "no gigabytes asked for");
        let mut slow = file.clone();
        slow[AT_COST + 4..AT_COST + 8].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(Head::read(&slow).err(), Some(Error::Damaged), "nor minutes");
        assert!(sane(crate::passcode::COST) && sane(super::super::COST) && sane(super::super::BACKUP_COST));
        assert!(!sane(Cost { m_kib: 19456, t: 2, p: 4 }) && !sane(Cost { m_kib: 64 * 1024 + 1, t: 3, p: 1 }));
        assert_ne!(mark_of(Some("$argon2id$a")), mark_of(Some("$argon2id$b")));
        assert_eq!(mark_of(None), [0; 8]);
    }
}
