use serde::{Deserialize, Serialize};

use crate::hlc::Hlc;
use crate::priority::Priority;
use crate::schedule::Schedule;

/// Version of the event format this app reads and writes. Events in a newer
/// format are kept without being applied (ADR 0005).
pub const FORMAT_VERSION: u32 = 3;

/// What the window says while a list holds events from a newer app.
pub const UPDATE_NOTICE: &str = "Update the app to see recent changes to this list";

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
    /// A repeating reminder was created, firing on its schedules, in the time
    /// zone `zone` or, if that is `None`, wherever the device is (floating).
    /// Format 2: an app that only reads format 1 keeps it without applying it,
    /// rather than mistaking it for a one-off.
    RecurringReminderCreated {
        reminder_id: String,
        title: String,
        schedules: Vec<Schedule>,
        zone: Option<String>,
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
    /// A reminder's setting was changed. Of two changes to the same setting,
    /// the one with the later `hlc` wins; the other stays in the history and
    /// can be restored by making the change again.
    ReminderEdited {
        reminder_id: String,
        hlc: Hlc,
        change: Change,
    },
    /// Someone closed the occurrence without doing it, with an optional note.
    OccurrenceSkipped {
        occurrence_id: String,
        skipped_at: i64,
        note: Option<String>,
    },
    /// The app closed the occurrence because it expired before anyone acted.
    OccurrenceMissed {
        occurrence_id: String,
        missed_at: i64,
    },
    /// Someone quieted the occurrence's alerts until `until`.
    OccurrenceSnoozed { occurrence_id: String, until: i64 },
    /// Someone silenced the occurrence's current alert.
    OccurrenceAcknowledged { occurrence_id: String },
    /// The list is called `name`. A standalone device's reminders come into
    /// an account as a list named after the device, and say so with this.
    /// Of several, the latest in the stream counts.
    ListNamed { name: String },
    /// The device that made this event is called `name`. Device names live
    /// here, in the user's encrypted personal list, so the server never
    /// reads them. The first device says it when it joins.
    DeviceNamed { name: String },
    /// The device that made this event has just signed in to the account,
    /// and is called `name`. The user's other devices show it as a notice.
    DeviceSignedIn { name: String },
    /// The device that made this event removed `device_id` from the account.
    /// The device's name stays, for the history; its sign-in notice goes.
    DeviceRemoved { device_id: String },
}

/// A reminder's settings, each of which is changed (and merged) on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    Title,
    FireAt,
    /// A free-text note. It is one value: concurrent edits never merge.
    Note,
    /// The schedule triggers, all of them as one value.
    Schedules,
    /// The time zone a reminder is pinned to, or floating.
    Zone,
    /// The reminder's priority.
    Priority,
    /// How long after its scheduled time an occurrence goes overdue, if the
    /// reminder overrides its priority's.
    Overdue,
    /// How long after its scheduled time an open occurrence is missed, if the
    /// reminder has such an expiry.
    Expiry,
}

/// A new value for one setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "setting", content = "value", rename_all = "snake_case")]
pub enum Change {
    Title(String),
    FireAt(i64),
    Note(String),
    Schedules(Vec<Schedule>),
    /// `None` is floating.
    Zone(Option<String>),
    Priority(Priority),
    /// Seconds after the scheduled time. `None` follows the priority.
    Overdue(Option<i64>),
    /// Seconds after the scheduled time. `None` is no delay-based expiry.
    Expiry(Option<i64>),
}

impl Change {
    pub fn setting(&self) -> Setting {
        match self {
            Change::Title(_) => Setting::Title,
            Change::FireAt(_) => Setting::FireAt,
            Change::Note(_) => Setting::Note,
            Change::Schedules(_) => Setting::Schedules,
            Change::Zone(_) => Setting::Zone,
            Change::Priority(_) => Setting::Priority,
            Change::Overdue(_) => Setting::Overdue,
            Change::Expiry(_) => Setting::Expiry,
        }
    }
}

impl Event {
    /// The event format this event needs a reader to understand. Most are
    /// still format 1, so apps that read only that keep working with them.
    pub fn format(&self) -> u32 {
        match self {
            Event::RecurringReminderCreated { .. }
            | Event::ReminderEdited {
                change: Change::Schedules(_) | Change::Zone(_),
                ..
            } => 2,
            // Priorities, overdue times and expiry arrived in format 3.
            Event::ReminderEdited {
                change: Change::Priority(_) | Change::Overdue(_) | Change::Expiry(_),
                ..
            } => 3,
            _ => 1,
        }
    }
}

/// An event with its place in its list's stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEvent {
    pub list_id: String,
    /// Position in the stream, from 1. A standalone device numbers its own
    /// streams. Once the device has joined a server, the server numbers
    /// them, and `None` means "not sent yet": the server hasn't numbered it.
    pub seq: Option<i64>,
    pub event_id: String,
    pub device_id: String,
    /// The user who made the change.
    pub author: String,
    /// When the device recorded it, in Unix seconds.
    pub recorded_at: i64,
    pub event: Event,
}

/// What travels inside an encrypted event: who, when, and the event itself.
/// The event stays raw JSON so one in a newer format can be kept untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    pub author: String,
    pub recorded_at: i64,
    pub event: serde_json::Value,
}

/// An event waiting to be sent to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub list_id: String,
    pub event_id: String,
    pub format: u32,
    pub recorded_at: i64,
    /// The JSON [`Payload`], which the sync layer encrypts and signs.
    pub payload: Vec<u8>,
}
