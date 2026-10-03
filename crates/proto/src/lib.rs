//! What the app and the sync server share: the OPAQUE cipher suite, the
//! account wire format, and the keys an account is made of.
//!
//! Binary fields travel as base64 inside JSON. Every encrypted or signed item
//! carries an algorithm identifier so the set can change later.

pub mod approval;
pub mod auth;
pub mod events;
pub mod keys;
pub mod opaque;
pub mod wire;

pub use approval::ApprovalKey;
pub use events::{open_list_key, seal_list_key, ListKey};
pub use keys::{sign_device, verify_device, DeviceKeys, KeyError, Keys};
pub use opaque::{argon2, argon2_with, Suite, ARGON_LANES, ARGON_MEMORY_KIB, ARGON_PASSES};
pub use opaque_ke;
