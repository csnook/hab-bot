//! The reminder core: an event stream in SQLite, and the state built by applying it.
//!
//! Every change is an [`Event`] appended to the personal list's stream. The current
//! [`State`] is never stored; it is rebuilt by applying the stream (ADR 0005), so
//! sync can later merge streams from several devices.

mod state;
mod store;

pub use state::{InboxItem, InboxSection, Occurrence, OccurrenceStatus, Reminder, State};
pub use store::Store;

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch, UTC.
pub type Millis = i64;

/// The only list in the walking skeleton.
pub const PERSONAL_LIST: &str = "personal";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("storage: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("event encoding: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("no open occurrence {0}")]
    NoOpenOccurrence(String),
    #[error("title must not be empty")]
    EmptyTitle,
}

pub type Result<T> = std::result::Result<T, Error>;

/// One recorded change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    ReminderCreated {
        reminder_id: String,
        title: String,
        /// When the one-off comes due.
        due_at: Millis,
        created_at: Millis,
    },
    /// A reminder fired: an occurrence opened. The identity of an occurrence is the
    /// reminder plus its scheduled time, so firings on several devices merge.
    OccurrenceFired {
        occurrence_id: String,
        reminder_id: String,
        scheduled_at: Millis,
        fired_at: Millis,
    },
    OccurrenceCompleted {
        occurrence_id: String,
        /// Who recorded it; the user, not the device.
        by: String,
        /// The recorded time of doing it.
        at: Millis,
    },
}

/// The deterministic identity of an occurrence.
pub fn occurrence_id(reminder_id: &str, scheduled_at: Millis) -> String {
    format!("{reminder_id}@{scheduled_at}")
}

/// The core: a store plus the state built from it.
pub struct Core {
    store: Store,
    state: State,
    user: String,
}

impl Core {
    /// Opens (or creates) the database at `path` and rebuilds state from its stream.
    pub fn open(path: &str, user: &str) -> Result<Core> {
        Self::with_store(Store::open(path)?, user)
    }

    pub fn open_in_memory(user: &str) -> Result<Core> {
        Self::with_store(Store::open_in_memory()?, user)
    }

    fn with_store(store: Store, user: &str) -> Result<Core> {
        let state = State::from_events(store.events(PERSONAL_LIST)?);
        Ok(Core {
            store,
            state,
            user: user.to_string(),
        })
    }

    fn record(&mut self, event: Event) -> Result<()> {
        self.store.append(PERSONAL_LIST, &event)?;
        self.state.apply(&event);
        Ok(())
    }

    /// Creates a one-off reminder in the personal list.
    pub fn create_one_off(&mut self, title: &str, due_at: Millis, now: Millis) -> Result<String> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::EmptyTitle);
        }
        let reminder_id = uuid::Uuid::new_v4().to_string();
        self.record(Event::ReminderCreated {
            reminder_id: reminder_id.clone(),
            title: title.to_string(),
            due_at,
            created_at: now,
        })?;
        Ok(reminder_id)
    }

    /// Fires every reminder that has come due and has not fired yet, including ones
    /// whose time passed while the app was closed. Returns the occurrences opened.
    pub fn fire_due(&mut self, now: Millis) -> Result<Vec<Occurrence>> {
        let due: Vec<(String, Millis)> = self
            .state
            .reminders()
            .filter(|r| !r.finished && r.due_at <= now)
            .filter(|r| !self.state.has_fired(&r.id, r.due_at))
            .map(|r| (r.id.clone(), r.due_at))
            .collect();
        let mut opened = Vec::new();
        for (reminder_id, scheduled_at) in due {
            let occurrence_id = occurrence_id(&reminder_id, scheduled_at);
            self.record(Event::OccurrenceFired {
                occurrence_id: occurrence_id.clone(),
                reminder_id,
                scheduled_at,
                fired_at: now,
            })?;
            opened.push(
                self.state
                    .occurrence(&occurrence_id)
                    .expect("just fired")
                    .clone(),
            );
        }
        Ok(opened)
    }

    /// Completes an open occurrence, recording who and when. A one-off is then finished.
    pub fn complete(&mut self, occurrence_id: &str, now: Millis) -> Result<()> {
        if !self.state.is_open(occurrence_id) {
            return Err(Error::NoOpenOccurrence(occurrence_id.to_string()));
        }
        self.record(Event::OccurrenceCompleted {
            occurrence_id: occurrence_id.to_string(),
            by: self.user.clone(),
            at: now,
        })
    }

    /// The Inbox: open occurrences, all under Due in the skeleton.
    pub fn inbox(&self) -> Vec<InboxItem> {
        self.state.inbox()
    }

    /// The earliest time at which an unfired reminder comes due, for scheduling the
    /// next wake-up.
    pub fn next_due(&self) -> Option<Millis> {
        self.state.next_due()
    }

    pub fn state(&self) -> &State {
        &self.state
    }
}

#[cfg(test)]
mod tests;
