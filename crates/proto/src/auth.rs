//! Device-authenticated requests.
//!
//! The server has no login yet beyond joining, so a device proves itself by
//! signing each request with its Ed25519 key. The server checks the signature
//! against the signing key it stored when the device joined. The signature
//! covers the method, path, time and a hash of the body, so a request can't be
//! changed or moved, and a captured one stops working after
//! [`MAX_REQUEST_AGE_SECS`].

use crate::keys::{DeviceKeys, KeyError};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

pub const HEADER_DEVICE: &str = "x-hab-device";
pub const HEADER_TIME: &str = "x-hab-time";
pub const HEADER_SIGNATURE: &str = "x-hab-signature";
pub const MAX_REQUEST_AGE_SECS: i64 = 300;

const REQUEST_STATEMENT: &[u8] = b"hab-bot request v1\n";

fn statement(device_id: i64, method: &str, path: &str, time: i64, body: &[u8]) -> Vec<u8> {
    let mut m = REQUEST_STATEMENT.to_vec();
    m.extend_from_slice(
        format!(
            "{device_id}\n{method}\n{path}\n{time}\n{}",
            hex(&Sha256::digest(body))
        )
        .as_bytes(),
    );
    m
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The base64 signature for the `x-hab-signature` header.
pub fn sign_request(
    device: &DeviceKeys,
    device_id: i64,
    method: &str,
    path: &str,
    time: i64,
    body: &[u8],
) -> String {
    let sig = device
        .signing
        .sign(&statement(device_id, method, path, time, body));
    STANDARD.encode(sig.to_bytes())
}

pub fn verify_request(
    signing_public: &[u8],
    device_id: i64,
    method: &str,
    path: &str,
    time: i64,
    body: &[u8],
    signature: &str,
) -> Result<(), KeyError> {
    let key: [u8; 32] = signing_public.try_into().map_err(|_| KeyError::Length)?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| KeyError::Length)?;
    let sig = STANDARD.decode(signature).map_err(|_| KeyError::Length)?;
    let sig: [u8; 64] = sig.as_slice().try_into().map_err(|_| KeyError::Length)?;
    key.verify(
        &statement(device_id, method, path, time, body),
        &Signature::from_bytes(&sig),
    )
    .map_err(|_| KeyError::Signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_request_verifies_only_as_sent() {
        let d = DeviceKeys::generate();
        let public = d.signing.verifying_key().to_bytes();
        let sig = sign_request(&d, 7, "POST", "/api/v1/x", 100, b"{}");
        verify_request(&public, 7, "POST", "/api/v1/x", 100, b"{}", &sig).unwrap();
        for (id, m, p, t, b) in [
            (8, "POST", "/api/v1/x", 100, &b"{}"[..]),
            (7, "GET", "/api/v1/x", 100, b"{}"),
            (7, "POST", "/api/v1/y", 100, b"{}"),
            (7, "POST", "/api/v1/x", 101, b"{}"),
            (7, "POST", "/api/v1/x", 100, b"{ }"),
        ] {
            assert!(verify_request(&public, id, m, p, t, b, &sig).is_err());
        }
        let other = DeviceKeys::generate().signing.verifying_key().to_bytes();
        assert!(verify_request(&other, 7, "POST", "/api/v1/x", 100, b"{}", &sig).is_err());
    }
}
