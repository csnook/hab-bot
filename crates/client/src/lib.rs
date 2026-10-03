//! The app side of joining a server: pinning its certificate, registering
//! with OPAQUE, making and keeping the account's keys, and judging passwords.
//! The window (Tauri and the UI) is a thin layer over this.

pub mod join;
pub mod keystore;
pub mod profile;
pub mod strength;
pub mod tls;

pub use join::{join, JoinError, JoinRequest, Joined};
pub use keystore::{KeyId, KeyKind, KeyStore, KeyStoreError};
pub use profile::{Profile, Setup, SetupFile};
pub use strength::{check_password, suggest_passphrase, PasswordCheck};
pub use tls::{probe, Pinned, ServerInfo, TlsError};
