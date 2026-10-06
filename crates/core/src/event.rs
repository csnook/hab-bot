use serde::{Deserialize, Serialize};

use crate::countdown::Countdown;
use crate::delay::{expiries, Delay};
use crate::hlc::Hlc;
use crate::pause::Pause;
use crate::priority::{AlertStyle, Priority};
use crate::schedule::Schedule;

/// Version of the event format this app reads and writes. Events in a newer
/// format are kept without being applied (ADR 0005).
pub const FORMAT_VERSION: u32 = 10;

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
    /// A countdown reminder was created: it fires `countdown` after its last
    /// occurrence closed. `last_done` is when it was last done, which starts
    /// the first countdown; `None` ("never") makes it fire at once. Format 5.
    CountdownReminderCreated {
        reminder_id: String,
        title: String,
        countdown: Countdown,
        zone: Option<String>,
        last_done: Option<i64>,
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
    /// Someone corrected how the occurrence was closed, to completed or
    /// skipped at `at`. It takes the place of the closings its author saw,
    /// `replaces`, which the history keeps; a closing made meanwhile on
    /// another device stands. Format 9.
    OccurrenceCorrected {
        occurrence_id: String,
        replaces: Vec<String>,
        kind: Correction,
        at: i64,
        note: Option<String>,
    },
    /// Someone took back the closings `replaces`, which their author saw. It
    /// says how that left the occurrence, as worked out where it was made.
    /// Format 9.
    OccurrenceUndone {
        occurrence_id: String,
        replaces: Vec<String>,
        outcome: UndoOutcome,
    },
    /// The occurrence fell in a pause and was skipped, because of it. If it
    /// wasn't opened yet (an instance that came due while paused) this
    /// opens it, closed; if it was open (when the pause began) it closes it.
    /// `until` is when the pause ends. It is dated by `skipped_at`, which is
    /// the instance's own time for one that came due, so that devices
    /// skipping the same instance agree. Format 10.
    OccurrenceSkippedForPause {
        occurrence_id: String,
        reminder_id: String,
        scheduled_at: i64,
        skipped_at: i64,
        until: Option<i64>,
        /// The pause is the list's, not the reminder's own.
        list: bool,
    },
    /// The app closed the occurrence because it expired before anyone acted.
    OccurrenceMissed {
        occurrence_id: String,
        missed_at: i64,
    },
    /// Someone quieted the occurrence's alerts until `until`.
    OccurrenceSnoozed { occurrence_id: String, until: i64 },
    /// Someone snoozed an expected occurrence ahead of time: it opens at
    /// `scheduled_at` as usual, quietly, and alerts when the snooze ends. Its
    /// id is the reminder plus the scheduled time, as for any occurrence.
    /// Format 6: an app that can't read it keeps it without applying it,
    /// rather than alerting at once.
    ExpectedOccurrenceSnoozed {
        reminder_id: String,
        scheduled_at: i64,
        until: i64,
    },
    /// Someone silenced the occurrence's current alert.
    OccurrenceAcknowledged { occurrence_id: String },
    /// The device that made this event alerted its user about the occurrence
    /// in `style`. Recorded for the first alert and each change of style, not
    /// for repeats. Format 4.
    OccurrenceAlerted {
        occurrence_id: String,
        style: AlertStyle,
    },
    /// The list is called `name`. A standalone device's reminders come into
    /// an account as a list named after the device, and say so with this.
    /// Of several, the latest in the stream counts.
    ListNamed { name: String },
    /// The list's colour, `#rrggbb`. Of several, the latest in the stream
    /// counts. Format 8.
    ListColoured { colour: String },
    /// The list was deleted. It is only gone while it holds no reminders: one
    /// that was moved or made into it meanwhile, on another device, keeps it
    /// (ADR 0009). Format 8.
    ListDeleted,
    /// The list is paused, or resumed with `None`: every reminder in it is set
    /// aside for the period (ADR 0011). Of several, the one with the latest
    /// `hlc` counts. Format 10.
    ListPaused { hlc: Hlc, pause: Option<Pause> },
    /// The reminder moved into this list from `from_list_id`, keeping its
    /// settings, occurrences and history, which this device finds in the list
    /// it came from. Of several moves of one reminder, the one with the latest
    /// `hlc` says where it is now. Format 8.
    ReminderMovedIn {
        reminder_id: String,
        from_list_id: String,
        hlc: Hlc,
    },
    /// The reminder was deleted but its history kept, marked deleted: it
    /// never fires or alerts again, on any device that has this event, and
    /// nothing that happened to it meanwhile on another device brings it
    /// back (ADR 0009). Format 8.
    ReminderDeleted { reminder_id: String },
    /// The reminder was deleted with its history: devices that have this
    /// event forget everything about it, including what other devices make
    /// of it afterwards. Format 8.
    ReminderPurged { reminder_id: String },
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

/// What a correction changes a closed occurrence to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Correction {
    Completed,
    Skipped,
}

/// How an undo left the occurrence, which the device that made it decides
/// (reopening is only right if it would still be open) so that every device
/// reaches the same answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum UndoOutcome {
    /// Open again.
    Reopened,
    /// Closed ahead of its time: it is expected again and fires at its time.
    Expected,
    /// It would no longer be open: missed, as of `at`.
    Missed { at: i64 },
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
    /// The countdown trigger of a countdown reminder.
    Countdown,
    /// Whether the reminder is paused, and until when.
    Pause,
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
    /// A delay from the scheduled time. `None` follows the priority.
    Overdue(Option<Delay>),
    /// The delays that mark an open occurrence missed, whichever comes first.
    /// Empty is no delay-based expiry.
    Expiry(#[serde(with = "expiries")] Vec<Delay>),
    Countdown(Countdown),
    /// `None` is not paused: resumed.
    Pause(Option<Pause>),
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
            Change::Countdown(_) => Setting::Countdown,
            Change::Pause(_) => Setting::Pause,
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
            // Overdue times and expiries that are the next time a schedule
            // matches, and several expiries, arrived in format 7.
            Event::ReminderEdited {
                change: Change::Overdue(Some(Delay::Next(_))),
                ..
            } => 7,
            Event::ReminderEdited {
                change: Change::Expiry(v),
                ..
            } if v.len() > 1 || v.iter().any(|d| !d.is_legacy()) => 7,
            // Priorities, overdue times and expiry arrived in format 3.
            Event::ReminderEdited {
                change: Change::Priority(_) | Change::Overdue(_) | Change::Expiry(_),
                ..
            } => 3,
            // Lists with colours, moving and deleting arrived in format 8.
            Event::ListColoured { .. }
            | Event::ListDeleted
            | Event::ReminderMovedIn { .. }
            | Event::ReminderDeleted { .. }
            | Event::ReminderPurged { .. } => 8,
            // Alerts arrived in format 4.
            Event::OccurrenceAlerted { .. } => 4,
            // Countdowns arrived in format 5.
            Event::CountdownReminderCreated { .. }
            | Event::ReminderEdited {
                change: Change::Countdown(_),
                ..
            } => 5,
            // Pausing arrived in format 10: an older app keeps these without
            // applying them, so it goes on firing and alerting until updated.
            Event::ListPaused { .. }
            | Event::OccurrenceSkippedForPause { .. }
            | Event::ReminderEdited {
                change: Change::Pause(_),
                ..
            } => 10,
            // Undoing and correcting arrived in format 9.
            Event::OccurrenceCorrected { .. } | Event::OccurrenceUndone { .. } => 9,
            // Snoozing ahead of time arrived in format 6.
            Event::ExpectedOccurrenceSnoozed { .. } => 6,
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
