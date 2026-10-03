//! The key two devices share while one approves the other (ADR 0004).
//!
//! The device that shows the QR code (or link) makes a random approval key and
//! puts it in the code, so it only ever travels from one screen to the other
//! camera, or by whatever the user pastes. The server relays the two devices'
//! messages but never has the key, so it can neither read them nor swap the
//! new device's keys for its own.

use crate::keys::KeyError;
use crate::wire::{Algs, ApprovalBlob, MAX_APPROVAL_BLOB};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use sha2::Sha256;
use zeroize::Zeroize;

const INFO: &[u8] = b"hab-bot approval v1";

#[derive(Clone, PartialEq, Eq)]
pub struct ApprovalKey([u8; 32]);

impl std::fmt::Debug for ApprovalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApprovalKey(..)")
    }
}

impl Drop for ApprovalKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl ApprovalKey {
    pub fn generate() -> ApprovalKey {
        let mut k = [0u8; 32];
        OsRng.fill_bytes(&mut k);
        ApprovalKey(k)
    }

    /// For links: URL-safe base64 without padding.
    pub fn to_text(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    pub fn from_text(text: &str) -> Result<ApprovalKey, KeyError> {
        let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| KeyError::Length)?;
        let k: [u8; 32] = bytes.as_slice().try_into().map_err(|_| KeyError::Length)?;
        Ok(ApprovalKey(k))
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        let mut key = [0u8; 32];
        Hkdf::<Sha256>::new(None, &self.0)
            .expand(INFO, &mut key)
            .expect("32 bytes is a valid length");
        let cipher = XChaCha20Poly1305::new((&key).into());
        key.zeroize();
        cipher
    }

    /// Seal `plain` for one `purpose` ("request" or "grant") of one approval,
    /// so a blob can't be moved to another approval or turned around.
    pub fn seal(&self, approval_id: &str, purpose: &str, plain: &[u8]) -> ApprovalBlob {
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher()
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plain,
                    aad: &aad(approval_id, purpose),
                },
            )
            .expect("encryption does not fail");
        ApprovalBlob {
            alg: Algs::APPROVAL.into(),
            nonce: nonce.to_vec(),
            ciphertext,
        }
    }

    pub fn open(
        &self,
        approval_id: &str,
        purpose: &str,
        blob: &ApprovalBlob,
    ) -> Result<Vec<u8>, KeyError> {
        if blob.alg != Algs::APPROVAL {
            return Err(KeyError::Algorithm(blob.alg.clone()));
        }
        if blob.nonce.len() != 24 || blob.ciphertext.len() > MAX_APPROVAL_BLOB {
            return Err(KeyError::Length);
        }
        self.cipher()
            .decrypt(
                XNonce::from_slice(&blob.nonce),
                Payload {
                    msg: &blob.ciphertext,
                    aad: &aad(approval_id, purpose),
                },
            )
            .map_err(|_| KeyError::Bundle)
    }
}

fn aad(approval_id: &str, purpose: &str) -> Vec<u8> {
    format!("{}\0{approval_id}\0{purpose}", Algs::APPROVAL).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blob_opens_only_with_its_key_approval_and_purpose() {
        let key = ApprovalKey::generate();
        let blob = key.seal("a1", "request", b"Laptop");
        assert_eq!(key.open("a1", "request", &blob).unwrap(), b"Laptop");
        assert!(key.open("a2", "request", &blob).is_err());
        assert!(key.open("a1", "grant", &blob).is_err());
        assert!(ApprovalKey::generate()
            .open("a1", "request", &blob)
            .is_err());
        let mut changed = blob.clone();
        changed.ciphertext[0] ^= 1;
        assert!(key.open("a1", "request", &changed).is_err());
    }

    #[test]
    fn a_key_survives_its_text() {
        let key = ApprovalKey::generate();
        assert_eq!(ApprovalKey::from_text(&key.to_text()).unwrap(), key);
        assert!(ApprovalKey::from_text("short").is_err());
        assert!(ApprovalKey::from_text("").is_err());
    }
}
