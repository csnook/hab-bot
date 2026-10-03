//! The sync server: one HTTPS listener, a self-signed certificate kept in its
//! SQLite file, a one-time setup code, and logs that never hold IP addresses
//! unless debug logging is on.

pub mod cert;
pub mod config;
pub mod db;
pub mod peers;
pub mod server;
pub mod setup;

pub use config::Config;
pub use server::{Server, StartError};
