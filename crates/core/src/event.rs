use serde::{Deserialize, Serialize};

/// Version of the stored event format. Later, the algorithm identifiers for
/// signing and encryption travel with it (ADR 0005).
pub const FORMAT_VERSION: u32 = 1;

/// One change to a reminder list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A one-off reminder was created, to fire at `fire_at`.
    ReminderCreated {
        reminder_id: String,
        title: String,
        fire_at: i64,
    },
    /// A reminder fired: an occurrence is open. Its id is the reminder plus
    /// the scheduled time, so firings on several devices merge.
    OccurrenceOpened {
        occurrence_id: String,
        reminder_id: String,
        scheduled_at: i64,
        fired_at: i64,
    },
    /// Someone recorded that the occurrence was done. Who is the event's author.
    OccurrenceCompleted {
        occurrence_id: String,
        completed_at: i64,
    },
}

/// An event with its place in its list's stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEvent {
    pub list_id: String,
    /// Position in the stream, from 1. A standalone device numbers its own
    /// streams; the server will later.
    pub seq: i64,
    pub event_id: String,
    pub device_id: String,
    /// The user who made the change.
    pub author: String,
    /// When the device recorded it, in Unix seconds.
    pub recorded_at: i64,
    pub event: Event,
}
