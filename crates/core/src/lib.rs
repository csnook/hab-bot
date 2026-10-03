//! The reminder core.
//!
//! Every change is an [`Event`] in a reminder list's stream, stored in SQLite
//! ([`Store`]). The current [`State`] is built by applying the stream in order
//! (ADR 0005). [`Core`] ties the two together and decides what fires.
//!
//! Time is always passed in as Unix seconds, so firing is deterministic and
//! testable. Events are neither signed nor encrypted yet.

mod core;
mod event;
mod state;
mod store;

pub use crate::core::{Core, Fired, Snapshot};
pub use event::{Event, StoredEvent, FORMAT_VERSION};
pub use state::{DueItem, Occurrence, Reminder, State, UpcomingItem};
pub use store::Store;

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
}

pub type Result<T> = std::result::Result<T, Error>;
