//! Pairing: a six-digit code on the handheld's screen, typed on the computer.
//!
//! 1. The app opens TLS to the handheld, taking whatever certificate it is
//!    given for now, and notes its fingerprint.
//! 2. It sends SPAKE2's first message, made from the code
//!    ([`Computer::start`]). The handheld answers with its own and its
//!    proof ([`handheld_answer`], [`confirm`] over *its* certificate).
//! 3. The app checks that proof over the certificate *it saw*. A wrong code
//!    and a different certificate (someone in the middle) both fail here,
//!    and the app sends nothing more.
//! 4. The app sends its own proof; the handheld checks it over its own
//!    certificate, and only then gives the token.
//!
//! What crosses the network tells a listener nothing to test codes
//! against; someone answering in the handheld's place gets one guess per
//! try, and the handheld lets a code be tried [`crate::TRIES`] times.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};

const COMPUTER: &[u8] = b"catalyst-companion computer";
const HANDHELD: &[u8] = b"catalyst-companion handheld";

/// Which end a proof is from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Side {
    Computer,
    Handheld,
}

/// The computer's half, between its message and the handheld's answer.
pub struct Computer(Spake2<Ed25519Group>);

impl Computer {
    /// Started with the code typed: this end and its first message.
    pub fn start(code: &str) -> (Computer, Vec<u8>) {
        let (state, msg) = Spake2::<Ed25519Group>::start_a(&Password::new(code.as_bytes()), &Identity::new(COMPUTER), &Identity::new(HANDHELD));
        (Computer(state), msg)
    }

    /// The key both ends now share (if the code was the same).
    pub fn finish(self, answer: &[u8]) -> Result<Vec<u8>, String> {
        self.0.finish(answer).map_err(|_| "the handheld's answer was not one".to_owned())
    }
}

/// The handheld's answer to the computer's first message, and the key.
pub fn handheld_answer(code: &str, message: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let (state, answer) = Spake2::<Ed25519Group>::start_b(&Password::new(code.as_bytes()), &Identity::new(COMPUTER), &Identity::new(HANDHELD));
    let key = state.finish(message).map_err(|_| "that was not a pairing message".to_owned())?;
    Ok((answer, key))
}

/// One end's proof that it holds `key`, over the certificate it is talking
/// through (its own, for the handheld; the one it saw, for the computer).
pub fn confirm(key: &[u8], side: Side, fingerprint: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key");
    mac.update(b"catalyst-companion confirm\0");
    mac.update(match side {
        Side::Computer => b"computer\0",
        Side::Handheld => b"handheld\0",
    });
    mac.update(fingerprint);
    mac.finalize().into_bytes().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_code_gives_the_same_key_and_the_proofs_match() {
        let fp = [7u8; 32];
        let (computer, first) = Computer::start("482913");
        let (answer, handheld_key) = handheld_answer("482913", &first).unwrap();
        let computer_key = computer.finish(&answer).unwrap();
        assert_eq!(computer_key, handheld_key);
        assert_eq!(confirm(&computer_key, Side::Handheld, &fp), confirm(&handheld_key, Side::Handheld, &fp));
        assert_ne!(confirm(&computer_key, Side::Handheld, &fp), confirm(&computer_key, Side::Computer, &fp), "each side's proof is its own");
    }

    #[test]
    fn a_wrong_code_or_another_certificate_fails_the_proof() {
        let fp = [7u8; 32];
        let (computer, first) = Computer::start("482913");
        let (answer, handheld_key) = handheld_answer("482914", &first).unwrap();
        let computer_key = computer.finish(&answer).unwrap();
        assert_ne!(confirm(&computer_key, Side::Handheld, &fp), confirm(&handheld_key, Side::Handheld, &fp), "a wrong code");
        let (computer, first) = Computer::start("482913");
        let (answer, handheld_key) = handheld_answer("482913", &first).unwrap();
        let computer_key = computer.finish(&answer).unwrap();
        assert_ne!(confirm(&computer_key, Side::Handheld, &[8u8; 32]), confirm(&handheld_key, Side::Handheld, &fp), "someone in the middle");
    }
}
