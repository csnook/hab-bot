//! Encrypting and signing events, and sealing list keys to devices.
//!
//! An event is encrypted with its list's key (XChaCha20-Poly1305) and signed
//! by the device that made it (Ed25519). The header (algorithms, format
//! version, list, event id, device, clock) is signed and also bound into the
//! encryption as associated data, so none of it can be changed without the
//! event failing to open. The server reads the header and verifies the
//! signature; it never has the key.
//!
//! A list key is sealed to each device with HPKE (RFC 9180) in auth mode, so a
//! device that opens a copy knows which of the user's devices sealed it. The
//! server stores the copies and could not make one that opens.

use crate::keys::{DeviceKeys, KeyError};
use crate::wire::{Algs, Envelope, SealedKey};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use hkdf::Hkdf;
use hpke::aead::ChaCha20Poly1305 as HpkeAead;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem as _, OpModeR, OpModeS, Serializable};
use rand::{rngs::OsRng, RngCore};
use sha2::Sha256;
use zeroize::Zeroize;

type Kem = X25519HkdfSha256;

const EVENT_SIGNATURE: &[u8] = b"hab-bot event signature v1\0";
const EVENT_HEADER: &[u8] = b"hab-bot event header v1\0";
const PERSONAL_INFO: &[u8] = b"hab-bot personal list key v1\0";
const SEAL_INFO: &[u8] = b"hab-bot list key v1\0";

/// A reminder list's symmetric key.
pub struct ListKey([u8; 32]);

impl Drop for ListKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Clone for ListKey {
    fn clone(&self) -> ListKey {
        ListKey(self.0)
    }
}

impl ListKey {
    pub fn random() -> ListKey {
        let mut k = [0u8; 32];
        OsRng.fill_bytes(&mut k);
        ListKey(k)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<ListKey, KeyError> {
        Ok(ListKey(bytes.try_into().map_err(|_| KeyError::Length)?))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The key of a user's personal list: the personal key, stretched with the
    /// list's id so the personal key itself is never used to encrypt events.
    pub fn personal(personal_key: &[u8; 32], list_id: &str) -> ListKey {
        let mut key = [0u8; 32];
        let mut info = PERSONAL_INFO.to_vec();
        info.extend_from_slice(list_id.as_bytes());
        Hkdf::<Sha256>::new(None, personal_key)
            .expand(&info, &mut key)
            .expect("32 bytes is a valid length");
        ListKey(key)
    }
}

fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

impl Envelope {
    /// The signed header, also the encryption's associated data.
    fn header(&self) -> Vec<u8> {
        let mut h = EVENT_HEADER.to_vec();
        put(&mut h, self.alg.as_bytes());
        h.extend_from_slice(&self.format.to_be_bytes());
        put(&mut h, self.list_id.as_bytes());
        put(&mut h, self.event_id.as_bytes());
        h.extend_from_slice(&self.device_id.to_be_bytes());
        h.extend_from_slice(&self.clock.to_be_bytes());
        h
    }

    fn signed_bytes(&self) -> Vec<u8> {
        let mut m = EVENT_SIGNATURE.to_vec();
        m.extend_from_slice(&self.header());
        put(&mut m, &self.nonce);
        put(&mut m, &self.ciphertext);
        m
    }

    /// Encrypt `plaintext` under the list key and sign it as `device`.
    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        key: &ListKey,
        device: &DeviceKeys,
        device_id: i64,
        list_id: &str,
        event_id: &str,
        format: u32,
        clock: i64,
        plaintext: &[u8],
    ) -> Envelope {
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut nonce);
        let mut e = Envelope {
            alg: Algs::EVENT.into(),
            format,
            list_id: list_id.into(),
            event_id: event_id.into(),
            device_id,
            clock,
            nonce: nonce.to_vec(),
            ciphertext: Vec::new(),
            signature: Vec::new(),
        };
        e.ciphertext = XChaCha20Poly1305::new((&key.0).into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &e.header(),
                },
            )
            .expect("encryption does not fail");
        e.signature = device.signing.sign(&e.signed_bytes()).to_bytes().to_vec();
        e
    }

    /// Check the device's signature. Does not need the list key, so the
    /// server does this too.
    pub fn verify(&self, signing_public: &[u8]) -> Result<(), KeyError> {
        if self.alg != Algs::EVENT {
            return Err(KeyError::Algorithm(self.alg.clone()));
        }
        let key: [u8; 32] = signing_public.try_into().map_err(|_| KeyError::Length)?;
        let key = VerifyingKey::from_bytes(&key).map_err(|_| KeyError::Length)?;
        let sig: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| KeyError::Length)?;
        key.verify(&self.signed_bytes(), &Signature::from_bytes(&sig))
            .map_err(|_| KeyError::Signature)
    }

    /// Decrypt the event. Fails for an algorithm this app doesn't know.
    pub fn open(&self, key: &ListKey) -> Result<Vec<u8>, KeyError> {
        if self.alg != Algs::EVENT {
            return Err(KeyError::Algorithm(self.alg.clone()));
        }
        if self.nonce.len() != 24 {
            return Err(KeyError::Length);
        }
        XChaCha20Poly1305::new((&key.0).into())
            .decrypt(
                XNonce::from_slice(&self.nonce),
                Payload {
                    msg: &self.ciphertext,
                    aad: &self.header(),
                },
            )
            .map_err(|_| KeyError::Bundle)
    }
}

fn seal_info(list_id: &str, key_version: u32) -> Vec<u8> {
    let mut info = SEAL_INFO.to_vec();
    put(&mut info, list_id.as_bytes());
    info.extend_from_slice(&key_version.to_be_bytes());
    info
}

fn sealing_pair(
    device: &DeviceKeys,
) -> Result<
    (
        <Kem as hpke::Kem>::PrivateKey,
        <Kem as hpke::Kem>::PublicKey,
    ),
    KeyError,
> {
    let sk = <Kem as hpke::Kem>::PrivateKey::from_bytes(&device.sealing.to_bytes())
        .map_err(|_| KeyError::Length)?;
    let pk = Kem::sk_to_pk(&sk);
    Ok((sk, pk))
}

/// Seal `key` to the device whose sealing public key is `recipient_public`,
/// authenticated as `sender`.
pub fn seal_list_key(
    key: &ListKey,
    list_id: &str,
    key_version: u32,
    sender: &DeviceKeys,
    sender_id: i64,
    recipient_id: i64,
    recipient_public: &[u8],
) -> Result<SealedKey, KeyError> {
    let recipient = <Kem as hpke::Kem>::PublicKey::from_bytes(recipient_public)
        .map_err(|_| KeyError::Length)?;
    let (encapped, ciphertext) = hpke::single_shot_seal::<HpkeAead, HkdfSha256, Kem, _>(
        &OpModeS::Auth(sealing_pair(sender)?),
        &recipient,
        &seal_info(list_id, key_version),
        &key.0,
        b"",
        &mut OsRng,
    )
    .map_err(|_| KeyError::Bundle)?;
    Ok(SealedKey {
        alg: Algs::LIST_KEY.into(),
        device_id: recipient_id,
        sealed_by: sender_id,
        key_version,
        encapped: encapped.to_bytes().to_vec(),
        ciphertext,
    })
}

/// Open a copy sealed to this device by the device whose sealing public key is
/// `sender_public`.
pub fn open_list_key(
    sealed: &SealedKey,
    list_id: &str,
    recipient: &DeviceKeys,
    sender_public: &[u8],
) -> Result<ListKey, KeyError> {
    if sealed.alg != Algs::LIST_KEY {
        return Err(KeyError::Algorithm(sealed.alg.clone()));
    }
    let sender =
        <Kem as hpke::Kem>::PublicKey::from_bytes(sender_public).map_err(|_| KeyError::Length)?;
    let encapped = <Kem as hpke::Kem>::EncappedKey::from_bytes(&sealed.encapped)
        .map_err(|_| KeyError::Length)?;
    let (sk, _) = sealing_pair(recipient)?;
    let mut plain = hpke::single_shot_open::<HpkeAead, HkdfSha256, Kem>(
        &OpModeR::Auth(sender),
        &sk,
        &encapped,
        &seal_info(list_id, sealed.key_version),
        &sealed.ciphertext,
        b"",
    )
    .map_err(|_| KeyError::Bundle)?;
    let key = ListKey::from_bytes(&plain);
    plain.zeroize();
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seal(key: &ListKey, d: &DeviceKeys, text: &[u8]) -> Envelope {
        Envelope::seal(key, d, 3, "list-1", "ev-1", 1, 1000, text)
    }

    #[test]
    fn an_event_opens_with_its_key_and_hides_its_content() {
        let key = ListKey::random();
        let d = DeviceKeys::generate();
        let e = seal(&key, &d, b"call the plumber");
        assert_eq!(e.alg, Algs::EVENT);
        e.verify(&d.signing.verifying_key().to_bytes()).unwrap();
        assert_eq!(e.open(&key).unwrap(), b"call the plumber");
        assert!(!e.ciphertext.windows(6).any(|w| w == b"plumber".as_slice()));
        assert!(e.open(&ListKey::random()).is_err());
    }

    #[test]
    fn changing_the_header_or_content_breaks_signature_and_encryption() {
        let key = ListKey::random();
        let d = DeviceKeys::generate();
        let public = d.signing.verifying_key().to_bytes();
        let e = seal(&key, &d, b"x");
        type Edit = Box<dyn Fn(&mut Envelope)>;
        let edits: Vec<Edit> = vec![
            Box::new(|e| e.format = 2),
            Box::new(|e| e.list_id = "list-2".into()),
            Box::new(|e| e.event_id = "ev-2".into()),
            Box::new(|e| e.device_id = 4),
            Box::new(|e| e.clock = 1001),
            Box::new(|e| e.ciphertext[0] ^= 1),
            Box::new(|e| e.nonce[0] ^= 1),
        ];
        for edit in edits {
            let mut f = e.clone();
            edit(&mut f);
            assert!(f.verify(&public).is_err());
            assert!(f.open(&key).is_err());
        }
        let other = DeviceKeys::generate().signing.verifying_key().to_bytes();
        assert!(e.verify(&other).is_err());
    }

    #[test]
    fn an_unknown_algorithm_is_named_not_guessed() {
        let key = ListKey::random();
        let d = DeviceKeys::generate();
        let mut e = seal(&key, &d, b"x");
        e.alg = "future/v9".into();
        assert_eq!(
            e.open(&key).err(),
            Some(KeyError::Algorithm("future/v9".into()))
        );
    }

    #[test]
    fn a_list_key_is_sealed_to_a_device_by_a_device() {
        let key = ListKey::random();
        let (a, b, stranger) = (
            DeviceKeys::generate(),
            DeviceKeys::generate(),
            DeviceKeys::generate(),
        );
        let sealed = seal_list_key(&key, "list-1", 1, &a, 1, 2, &b.sealing_public()).unwrap();
        assert_eq!((sealed.sealed_by, sealed.device_id), (1, 2));
        let opened = open_list_key(&sealed, "list-1", &b, &a.sealing_public()).unwrap();
        assert_eq!(opened.as_bytes(), key.as_bytes());
        // Only the recipient opens it, only from the real sender, only for this list.
        assert!(open_list_key(&sealed, "list-1", &stranger, &a.sealing_public()).is_err());
        assert!(open_list_key(&sealed, "list-1", &b, &stranger.sealing_public()).is_err());
        assert!(open_list_key(&sealed, "list-2", &b, &a.sealing_public()).is_err());
        assert!(!sealed
            .ciphertext
            .windows(32)
            .any(|w| w == key.as_bytes().as_slice()));
    }

    #[test]
    fn the_personal_list_key_depends_on_the_personal_key_and_the_list() {
        let (p, q) = ([1u8; 32], [2u8; 32]);
        let k = ListKey::personal(&p, "a");
        assert_eq!(k.as_bytes(), ListKey::personal(&p, "a").as_bytes());
        assert_ne!(k.as_bytes(), ListKey::personal(&p, "b").as_bytes());
        assert_ne!(k.as_bytes(), ListKey::personal(&q, "a").as_bytes());
        assert_ne!(k.as_bytes(), &p);
    }
}
