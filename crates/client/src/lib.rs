//! The app side of joining a server: pinning its certificate, registering
//! with OPAQUE, making and keeping the account's keys, and judging passwords.
//! The window (Tauri and the UI) is a thin layer over this.

pub mod approve;
pub mod code;
pub mod join;
pub mod keystore;
pub mod profile;
pub mod signin;
pub mod strength;
pub mod sync;
pub mod tls;

pub use approve::{ApprovalLink, ApproveError, NewDevice};
pub use code::{parse_target, CodeError, SignInCode, SignInTarget};
pub use join::{join, JoinError, JoinRequest, Joined};
pub use keystore::{KeyId, KeyKind, KeyStore, KeyStoreError};
pub use profile::{Profile, Setup, SetupFile};
pub use signin::{sign_in, SignInError, SignInRequest, SignedIn};
pub use strength::{check_password, suggest_passphrase, PasswordCheck};
pub use sync::{DeviceInfo, PendingDevice, SyncError, SyncStatus, Syncer};
pub use tls::{probe, Pinned, ServerInfo, TlsError};
