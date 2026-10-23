//! Pairing two devices by a short code and sealing what they send each other.
//!
//! The code never travels: both sides run SPAKE2 with it, so someone listening on the network
//! learns nothing to guess the code with offline, and a wrong guess costs a whole pairing session.
//! Each side proves it holds the key before the other keeps it.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};

type HmacSha256 = Hmac<Sha256>;

const NONCE: usize = 12;
const TAG: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairError {
    /// The two sides used different codes, or someone is guessing.
    WrongCode,
    /// The other side's message was not a pairing message.
    Garbled,
}

fn derive(shared: &[u8], label: &[u8]) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(shared).expect("any key length");
    mac.update(label);
    mac.finalize().into_bytes().into()
}

fn check(shared: &[u8], label: &[u8], proof: &[u8]) -> Result<(), PairError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(shared).expect("any key length");
    mac.update(label);
    mac.verify_slice(proof).map_err(|_| PairError::WrongCode)
}

const KEY_LABEL: &[u8] = b"ninari devices v1 key";
const HOST_PROOF: &[u8] = b"ninari devices v1 host";
const JOINER_PROOF: &[u8] = b"ninari devices v1 joiner";

/// What the host sends back: its SPAKE2 message and proof that it got the same key.
#[derive(Debug, Clone)]
pub struct Answer {
    pub msg: Vec<u8>,
    pub proof: [u8; 32],
}

/// The device the code is typed into.
pub struct Joiner(Spake2<Ed25519Group>);

impl Joiner {
    pub fn start(code: &str, joiner_id: &str, host_id: &str) -> (Self, Vec<u8>) {
        let (state, msg) = Spake2::<Ed25519Group>::start_a(
            &Password::new(code.as_bytes()),
            &Identity::new(joiner_id.as_bytes()),
            &Identity::new(host_id.as_bytes()),
        );
        (Self(state), msg)
    }

    /// The shared key and the proof to send back, if the host used the same code.
    pub fn finish(self, answer: &Answer) -> Result<([u8; 32], [u8; 32]), PairError> {
        let shared = self.0.finish(&answer.msg).map_err(|_| PairError::Garbled)?;
        check(&shared, HOST_PROOF, &answer.proof)?;
        Ok((derive(&shared, KEY_LABEL), derive(&shared, JOINER_PROOF)))
    }
}

/// The device that shows the code, waiting for the joiner's proof.
pub struct Host {
    shared: Vec<u8>,
}

impl Host {
    pub fn answer(
        code: &str,
        joiner_id: &str,
        host_id: &str,
        hello: &[u8],
    ) -> Result<(Self, Answer), PairError> {
        let (state, msg) = Spake2::<Ed25519Group>::start_b(
            &Password::new(code.as_bytes()),
            &Identity::new(joiner_id.as_bytes()),
            &Identity::new(host_id.as_bytes()),
        );
        let shared = state.finish(hello).map_err(|_| PairError::Garbled)?;
        let proof = derive(&shared, HOST_PROOF);
        Ok((Self { shared }, Answer { msg, proof }))
    }

    pub fn finish(self, proof: &[u8]) -> Result<[u8; 32], PairError> {
        check(&self.shared, JOINER_PROOF, proof)?;
        Ok(derive(&self.shared, KEY_LABEL))
    }
}

/// Encrypts and authenticates `plain`; `sender` is bound in, so a message cannot be passed off
/// as another device's. A fresh random nonce leads the result.
pub fn seal(key: &[u8; 32], sender: &[u8], plain: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce: [u8; NONCE] = rand::random();
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: sender,
            },
        )
        .expect("encrypting into memory does not fail");
    [nonce.as_slice(), &sealed].concat()
}

pub fn open(key: &[u8; 32], sender: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < NONCE + TAG {
        return None;
    }
    let (nonce, body) = sealed.split_at(NONCE);
    ChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: body,
                aad: sender,
            },
        )
        .ok()
}

pub fn new_code() -> String {
    format!("{:06}", rand::random_range(0..1_000_000u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(host_code: &str, joiner_code: &str) -> Result<([u8; 32], [u8; 32]), PairError> {
        let (joiner, hello) = Joiner::start(joiner_code, "pc-b", "pc-a");
        let (host, answer) = Host::answer(host_code, "pc-b", "pc-a", &hello)?;
        let (joiner_key, proof) = joiner.finish(&answer)?;
        let host_key = host.finish(&proof)?;
        Ok((joiner_key, host_key))
    }

    #[test]
    fn the_same_code_gives_both_sides_the_same_key() {
        let (a, b) = pair("482915", "482915").unwrap();
        assert_eq!(a, b);
        let (c, _) = pair("482915", "482915").unwrap();
        assert_ne!(a, c, "every pairing makes a new key");
    }

    #[test]
    fn a_wrong_code_is_caught_before_any_key_is_kept() {
        assert_eq!(pair("482915", "482916"), Err(PairError::WrongCode));
    }

    #[test]
    fn a_forged_proof_does_not_make_the_host_keep_the_key() {
        let (_, hello) = Joiner::start("111111", "pc-b", "pc-a");
        let (host, _) = Host::answer("111111", "pc-b", "pc-a", &hello).unwrap();
        assert_eq!(host.finish(&[0u8; 32]), Err(PairError::WrongCode));
    }

    #[test]
    fn sealed_messages_open_only_with_the_key_and_the_sender_they_were_sealed_for() {
        let key = [7u8; 32];
        let sealed = seal(&key, b"pc-b", b"{\"since\":0}");
        assert_eq!(
            open(&key, b"pc-b", &sealed).as_deref(),
            Some(&b"{\"since\":0}"[..])
        );
        assert_eq!(open(&[8u8; 32], b"pc-b", &sealed), None);
        assert_eq!(open(&key, b"pc-c", &sealed), None);
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(open(&key, b"pc-b", &tampered), None);
        assert_ne!(
            seal(&key, b"pc-b", b"x"),
            seal(&key, b"pc-b", b"x"),
            "a fresh nonce every time"
        );
        assert_eq!(open(&key, b"pc-b", b"short"), None);
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..50 {
            let c = new_code();
            assert_eq!(c.len(), 6);
            assert!(c.chars().all(|ch| ch.is_ascii_digit()));
        }
    }
}
