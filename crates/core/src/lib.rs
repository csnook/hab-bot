//! The reminder core.
//!
//! Every change is an [`Event`] in a reminder list's stream, stored in SQLite
//! ([`Store`]). The current [`State`] is built by applying the stream in order
//! (ADR 0005). [`Core`] ties the two together and decides what fires.
//!
//! Time is always passed in as Unix seconds, so firing is deterministic and
//! testable. Signing, encryption and talking to the server belong to the
//! client; the core only keeps what it needs to be synced: which events the
//! server has numbered, which it hasn't, and which are in a format it can't read.

mod core;
mod event;
mod state;
mod store;

pub use crate::core::{Core, Fired, SignInNotice, Snapshot};
pub use event::{Event, Outgoing, Payload, StoredEvent, FORMAT_VERSION, UPDATE_NOTICE};
pub use state::{DueItem, Occurrence, Reminder, SignIn, State, UpcomingItem};
pub use store::{HeldEvent, Store};

/// Errors from the core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("storage error: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("a stored event could not be read: {0}")]
    Corrupt(#[from] serde_json::Error),
    #[error("the title is empty")]
    EmptyTitle,
    #[error("there is no open occurrence {0}")]
    NotOpen(String),
    #[error("an event from the server could not be read: {0}")]
    BadEvent(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
