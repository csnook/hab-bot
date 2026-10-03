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
    /// Event: XChaCha20-Poly1305 under the list key, signed with Ed25519.
    pub const EVENT: &'static str = "xchacha20poly1305+ed25519/v1";
    /// List key: HPKE (RFC 9180) in auth mode, DHKEM(X25519, HKDF-SHA256),
    /// HKDF-SHA256 and ChaCha20-Poly1305.
    pub const LIST_KEY: &'static str = "hpke-auth-x25519-hkdf-sha256-chacha20poly1305/v1";
}

pub(crate) mod b64 {
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
    // The device's name is not here: the server can't read it. It is kept in
    // the user's encrypted settings, as an event in the personal list.
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

/// Signing in, step 1: the first OPAQUE message for a username.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoginStart {
    pub username: String,
    #[serde(with = "b64")]
    pub credential_request: Vec<u8>,
}

/// The server's answer. For a username it doesn't know it answers the same
/// way, so the answer doesn't say which usernames exist.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoginStarted {
    /// Names this sign-in attempt in the next two calls.
    pub login_id: String,
    #[serde(with = "b64")]
    pub credential_response: Vec<u8>,
    /// The Argon2id cost the account's password was registered with.
    pub kdf: Kdf,
}

/// Step 2: the last OPAQUE message. If it checks out, the server releases the
/// key bundle.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoginFinish {
    pub login_id: String,
    #[serde(with = "b64")]
    pub credential_finalization: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoginFinished {
    pub display_name: String,
    pub admin: bool,
    #[serde(with = "b64")]
    pub identity_public: Vec<u8>,
    pub bundle: KeyBundle,
}

/// Step 3: the new device, signed by the identity key it unlocked. Only a
/// sign-in that passed step 2 may add one, once.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoginDevice {
    pub login_id: String,
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

// ---- Sync: the encrypted event log (ADR 0005) ----

/// The server accepts events whose clock is at most this far ahead of its own.
pub const MAX_CLOCK_AHEAD_SECS: i64 = 600;
/// The largest ciphertext the server takes for one event.
pub const MAX_EVENT_BYTES: usize = 64 * 1024;
/// The longest id a device may make.
pub const MAX_ID: usize = 80;

/// Ids that devices make (lists, events) are short and plain, so they are safe
/// to log, store and put in a signed header.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '@' | '.'))
}

/// One event as the server holds it. The server reads the header, which the
/// device signed, and never the ciphertext.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// Algorithms used to encrypt and sign: [`Algs::EVENT`].
    pub alg: String,
    /// Version of the event format inside. A device that doesn't know it keeps
    /// the event without applying it; the server accepts any.
    pub format: u32,
    pub list_id: String,
    pub event_id: String,
    pub device_id: i64,
    /// The device's clock when it made the event, in Unix seconds. The server
    /// rejects clocks set far ahead of its own.
    pub clock: i64,
    #[serde(with = "b64")]
    pub nonce: Vec<u8>,
    #[serde(with = "b64")]
    pub ciphertext: Vec<u8>,
    /// The device's Ed25519 signature over the header and the ciphertext.
    #[serde(with = "b64")]
    pub signature: Vec<u8>,
}

/// A list key sealed to one device.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SealedKey {
    pub alg: String,
    /// The device this copy is for.
    pub device_id: i64,
    /// The device that sealed it (HPKE auth mode proves who).
    pub sealed_by: i64,
    pub key_version: u32,
    #[serde(with = "b64")]
    pub encapped: Vec<u8>,
    #[serde(with = "b64")]
    pub ciphertext: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RegisterList {
    pub list_id: String,
    pub keys: Vec<SealedKey>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListRef {
    pub list_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ListRefs {
    pub lists: Vec<ListRef>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SealedKeys {
    pub keys: Vec<SealedKey>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeviceEntry {
    pub id: i64,
    pub record: DeviceRecord,
    /// When the server last heard from the device (a request or a ping on its
    /// WebSocket), in Unix seconds. The server sees timing, never content.
    #[serde(default)]
    pub last_synced: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeviceList {
    /// The devices that are on the account now.
    pub devices: Vec<DeviceEntry>,
    /// Devices that were removed. Their records are kept so that the events
    /// they made before removal still verify. Never seal a key to one.
    #[serde(default)]
    pub retired: Vec<DeviceEntry>,
}

/// The new keys of one list when a device is removed: every version of the
/// list's key, sealed by the removing device to each device that remains. The
/// newest version is the new one, and the older ones come along so that a
/// device added later, or one whose copies were sealed by the removed device,
/// can still read the whole history.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Rotation {
    pub list_id: String,
    pub keys: Vec<SealedKey>,
}

/// Remove a device from the caller's account and rotate every list's key in
/// the same step, so there is no moment when the device is gone and the old
/// key is still the newest.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RemoveDevice {
    pub device_id: i64,
    pub lists: Vec<Rotation>,
}

/// What the server did with an event it was sent.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Numbered {
    pub event_id: String,
    pub seq: i64,
    pub received_at: i64,
    /// True when the server already had this exact event (a resend).
    pub duplicate: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub event_id: String,
    pub error: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AppendBatch {
    pub envelopes: Vec<Envelope>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AppendResults {
    pub numbered: Vec<Numbered>,
    pub rejected: Vec<Rejected>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FetchEvents {
    pub list_id: String,
    /// Return events numbered after this one.
    pub after: i64,
    pub limit: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NumberedEnvelope {
    pub seq: i64,
    pub received_at: i64,
    pub envelope: Envelope,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EventPage {
    pub events: Vec<NumberedEnvelope>,
    pub more: bool,
}

/// What a device says on its WebSocket.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ClientMessage {
    Append { envelope: Envelope },
}

/// What the server says on a device's WebSocket.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ServerMessage {
    /// The device's own event was numbered.
    Numbered(Numbered),
    Rejected(Rejected),
    /// Another device's event, numbered.
    Event {
        seq: i64,
        received_at: i64,
        envelope: Envelope,
    },
    /// The device missed pushes; it should download what it lacks.
    Resync,
    /// Another device was removed and the lists' keys were rotated: fetch the
    /// device list and the new keys.
    KeysChanged,
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
