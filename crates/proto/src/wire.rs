//! The JSON the app and server exchange for joining.

use serde::{Deserialize, Serialize};

pub struct Algs;

impl Algs {
    /// Bundle: XChaCha20-Poly1305 under HKDF-SHA256 of OPAQUE's export key.
    pub const BUNDLE: &'static str = "xchacha20poly1305+hkdf-sha256/v1";
    /// Device: Ed25519 signing key and X25519 sealing key, signed by Ed25519.
    pub const DEVICE: &'static str = "ed25519+x25519/v1";
    /// Identity: Ed25519.
    pub const IDENTITY: &'static str = "ed25519/v1";
    /// OPAQUE: ristretto255, 3DH, SHA-512.
    pub const OPAQUE: &'static str = "opaque-ristretto255-sha512/v1";
}

mod b64 {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

/// Step 1: the first OPAQUE message, with the setup code that allows it.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JoinStart {
    pub setup_code: String,
    pub username: String,
    #[serde(with = "b64")]
    pub registration_request: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JoinStarted {
    #[serde(with = "b64")]
    pub registration_response: Vec<u8>,
}

/// The cost the client used for Argon2id, kept with the bundle.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Kdf {
    pub alg: String,
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyBundle {
    pub alg: String,
    #[serde(with = "b64")]
    pub nonce: Vec<u8>,
    #[serde(with = "b64")]
    pub ciphertext: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeviceRecord {
    pub alg: String,
    pub name: String,
    pub portable: bool,
    #[serde(with = "b64")]
    pub signing_public: Vec<u8>,
    #[serde(with = "b64")]
    pub sealing_public: Vec<u8>,
    /// The identity key's signature over both public keys.
    #[serde(with = "b64")]
    pub signature: Vec<u8>,
}

/// Step 2: everything the first account is made of. The setup code is checked
/// again and the account is created in the same transaction.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JoinFinish {
    pub setup_code: String,
    pub username: String,
    pub display_name: String,
    #[serde(with = "b64")]
    pub registration_upload: Vec<u8>,
    pub identity_alg: String,
    #[serde(with = "b64")]
    pub identity_public: Vec<u8>,
    pub kdf: Kdf,
    pub bundle: KeyBundle,
    pub device: DeviceRecord,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Joined {
    pub account_id: i64,
    pub device_id: i64,
    pub admin: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ErrorBody {
    pub error: String,
}

pub const MAX_USERNAME: usize = 32;
pub const MAX_DISPLAY_NAME: usize = 64;

/// Usernames are 1 to 32 characters of lower-case letters, digits, `.`, `_`
/// and `-`, starting with a letter or digit.
pub fn valid_username(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.len() <= MAX_USERNAME
        && chars
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

pub fn valid_display_name(name: &str) -> bool {
    let t = name.trim();
    !t.is_empty() && t.chars().count() <= MAX_DISPLAY_NAME && !t.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert!(valid_username("chris"));
        assert!(valid_username("a.b_c-9"));
        assert!(!valid_username(""));
        assert!(!valid_username("Chris"));
        assert!(!valid_username("-a"));
        assert!(!valid_username("a b"));
        assert!(!valid_username(&"a".repeat(33)));
    }

    #[test]
    fn display_names() {
        assert!(valid_display_name("Chris S"));
        assert!(!valid_display_name("   "));
        assert!(!valid_display_name("a\nb"));
        assert!(!valid_display_name(&"x".repeat(65)));
    }
}
