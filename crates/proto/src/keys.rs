//! The keys an account is made of, and the bundle that carries them.
//!
//! The **identity key** signs the user's devices. The **personal key** encrypts
//! the personal list. Both live in a **bundle** on the server, encrypted under
//! a key derived from OPAQUE's export key. Each **device** has its own signing
//! key and sealing key, and the identity key signs both.

use crate::wire::{Algs, DeviceRecord, KeyBundle};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyError {
    #[error("the key bundle could not be opened")]
    Bundle,
    #[error("unknown algorithm {0}")]
    Algorithm(String),
    #[error("the device is not signed by this identity")]
    Signature,
    #[error("a key has the wrong length")]
    Length,
}

const BUNDLE_INFO: &[u8] = b"hab-bot key bundle v1";
const DEVICE_STATEMENT: &[u8] = b"hab-bot device v1";

/// The user's identity and personal keys.
pub struct Keys {
    pub identity: SigningKey,
    pub personal: [u8; 32],
}

impl Drop for Keys {
    fn drop(&mut self) {
        self.personal.zeroize();
    }
}

impl Keys {
    pub fn generate() -> Keys {
        let mut personal = [0u8; 32];
        OsRng.fill_bytes(&mut personal);
        Keys {
            identity: SigningKey::generate(&mut OsRng),
            personal,
        }
    }

    pub fn identity_public(&self) -> [u8; 32] {
        self.identity.verifying_key().to_bytes()
    }

    /// 64 bytes: the identity secret, then the personal key.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.identity.to_bytes().to_vec();
        out.extend_from_slice(&self.personal);
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Keys, KeyError> {
        if bytes.len() != 64 {
            return Err(KeyError::Length);
        }
        let identity: [u8; 32] = bytes[..32].try_into().unwrap();
        let personal: [u8; 32] = bytes[32..].try_into().unwrap();
        Ok(Keys {
            identity: SigningKey::from_bytes(&identity),
            personal,
        })
    }

    /// Encrypt the keys under a key derived from OPAQUE's export key. The
    /// username is bound in, so a bundle can't be moved to another account.
    pub fn seal(&self, export_key: &[u8], username: &str) -> KeyBundle {
        let mut plain = self.to_bytes();
        let cipher = bundle_cipher(export_key);
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut nonce);
        let alg = Algs::BUNDLE;
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: &bundle_aad(alg, username),
                },
            )
            .expect("encryption does not fail");
        plain.zeroize();
        KeyBundle {
            alg: alg.into(),
            nonce: nonce.to_vec(),
            ciphertext,
        }
    }

    pub fn open(bundle: &KeyBundle, export_key: &[u8], username: &str) -> Result<Keys, KeyError> {
        if bundle.alg != Algs::BUNDLE {
            return Err(KeyError::Algorithm(bundle.alg.clone()));
        }
        if bundle.nonce.len() != 24 {
            return Err(KeyError::Bundle);
        }
        let mut plain = bundle_cipher(export_key)
            .decrypt(
                XNonce::from_slice(&bundle.nonce),
                Payload {
                    msg: &bundle.ciphertext,
                    aad: &bundle_aad(&bundle.alg, username),
                },
            )
            .map_err(|_| KeyError::Bundle)?;
        let keys = Keys::from_bytes(&plain);
        plain.zeroize();
        keys
    }
}

fn bundle_cipher(export_key: &[u8]) -> XChaCha20Poly1305 {
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(None, export_key)
        .expand(BUNDLE_INFO, &mut key)
        .expect("32 bytes is a valid length");
    let cipher = XChaCha20Poly1305::new((&key).into());
    key.zeroize();
    cipher
}

fn bundle_aad(alg: &str, username: &str) -> Vec<u8> {
    format!("{alg}\0{username}").into_bytes()
}

/// One device's own keys: one to sign events, one to receive sealed list keys.
pub struct DeviceKeys {
    pub signing: SigningKey,
    pub sealing: StaticSecret,
}

impl DeviceKeys {
    pub fn generate() -> DeviceKeys {
        DeviceKeys {
            signing: SigningKey::generate(&mut OsRng),
            sealing: StaticSecret::random_from_rng(OsRng),
        }
    }

    pub fn sealing_public(&self) -> [u8; 32] {
        PublicKey::from(&self.sealing).to_bytes()
    }

    /// 64 bytes: the signing secret, then the sealing secret.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.signing.to_bytes().to_vec();
        out.extend_from_slice(&self.sealing.to_bytes());
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<DeviceKeys, KeyError> {
        if bytes.len() != 64 {
            return Err(KeyError::Length);
        }
        let signing: [u8; 32] = bytes[..32].try_into().unwrap();
        let sealing: [u8; 32] = bytes[32..].try_into().unwrap();
        Ok(DeviceKeys {
            signing: SigningKey::from_bytes(&signing),
            sealing: StaticSecret::from(sealing),
        })
    }

    /// This device's public keys, signed by the identity key.
    pub fn record(&self, identity: &SigningKey, portable: bool) -> DeviceRecord {
        let signing_public = self.signing.verifying_key().to_bytes();
        let sealing_public = self.sealing_public();
        let signature = identity
            .sign(&device_statement(&signing_public, &sealing_public))
            .to_bytes();
        DeviceRecord {
            alg: Algs::DEVICE.into(),
            portable,
            signing_public: signing_public.to_vec(),
            sealing_public: sealing_public.to_vec(),
            signature: signature.to_vec(),
        }
    }
}

fn device_statement(signing_public: &[u8], sealing_public: &[u8]) -> Vec<u8> {
    let mut m = DEVICE_STATEMENT.to_vec();
    m.extend_from_slice(signing_public);
    m.extend_from_slice(sealing_public);
    m
}

/// Check that `identity_public` signed the device's keys.
pub fn verify_device(identity_public: &[u8], device: &DeviceRecord) -> Result<(), KeyError> {
    if device.alg != Algs::DEVICE {
        return Err(KeyError::Algorithm(device.alg.clone()));
    }
    let identity: [u8; 32] = identity_public.try_into().map_err(|_| KeyError::Length)?;
    let identity = VerifyingKey::from_bytes(&identity).map_err(|_| KeyError::Length)?;
    if device.signing_public.len() != 32 || device.sealing_public.len() != 32 {
        return Err(KeyError::Length);
    }
    let signature: [u8; 64] = device
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| KeyError::Length)?;
    identity
        .verify(
            &device_statement(&device.signing_public, &device.sealing_public),
            &Signature::from_bytes(&signature),
        )
        .map_err(|_| KeyError::Signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_opens_only_with_its_export_key_and_username() {
        let keys = Keys::generate();
        let bundle = keys.seal(b"export key", "chris");
        let back = Keys::open(&bundle, b"export key", "chris").unwrap();
        assert_eq!(back.personal, keys.personal);
        assert_eq!(back.identity_public(), keys.identity_public());
        assert_eq!(
            Keys::open(&bundle, b"other key", "chris").err(),
            Some(KeyError::Bundle)
        );
        assert_eq!(
            Keys::open(&bundle, b"export key", "mallory").err(),
            Some(KeyError::Bundle)
        );
        assert!(!bundle
            .ciphertext
            .windows(32)
            .any(|w| w == keys.personal.as_slice()));
    }

    #[test]
    fn a_device_is_vouched_for_by_the_identity_that_signed_it() {
        let keys = Keys::generate();
        let device = DeviceKeys::generate().record(&keys.identity, false);
        verify_device(&keys.identity_public(), &device).unwrap();

        let other = Keys::generate();
        assert_eq!(
            verify_device(&other.identity_public(), &device),
            Err(KeyError::Signature)
        );
        let mut swapped = device.clone();
        swapped.sealing_public = DeviceKeys::generate().sealing_public().to_vec();
        assert_eq!(
            verify_device(&keys.identity_public(), &swapped),
            Err(KeyError::Signature)
        );
    }

    #[test]
    fn keys_round_trip_through_bytes() {
        let d = DeviceKeys::generate();
        let back = DeviceKeys::from_bytes(&d.to_bytes()).unwrap();
        assert_eq!(back.sealing_public(), d.sealing_public());
        assert_eq!(back.signing.to_bytes(), d.signing.to_bytes());
        assert_eq!(Keys::from_bytes(&[0; 3]).err(), Some(KeyError::Length));
    }
}
