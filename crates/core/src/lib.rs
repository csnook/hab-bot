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

mod alerter;
mod core;
mod countdown;
mod delay;
mod event;
mod hlc;
mod pause;
mod priority;
mod schedule;
mod state;
mod store;

pub use crate::core::{
    ClosedEntry, ClosedView, Core, CountdownItem, DeletedReminder, DeviceNotice, EarlierItem,
    EditReminder, ExpectedItem, Filters, Fired, Inbox, ListInfo, OccurrenceView, Outcome,
    PausedOpen, PausedReminder, ReconciliationNotice, ReminderView, ScheduleView, SecurityNotice,
    SignInNotice, Snapshot, SnoozeKind, SnoozeOption, SnoozePicker, TriggerView,
};
pub use alerter::{
    Alerter, Command, NoServer, Notification, ServerCheck, Urgency, ACTION_ACKNOWLEDGE,
    ACTION_DONE, ACTION_OPEN, ACTION_SKIP, ACTION_SNOOZE,
};
pub use countdown::{Countdown, CountdownUnit};
pub use delay::{Delay, DelaySpec};
pub use event::{
    Change, Correction, Event, Outgoing, Payload, Setting, StoredEvent, UndoOutcome,
    FORMAT_VERSION, UPDATE_NOTICE,
};
pub use hlc::{Hlc, MAX_AHEAD_MS};
pub use pause::{Pause, PauseCause};
pub use priority::{
    built_in_priorities, AlertStyle, EscalationStep, Priority, PriorityInfo, PrioritySettings,
};
pub use schedule::{clock_time, day_bounds, system_zone_name, zone, Parts, Pattern, Schedule};
pub use state::{
    last_chance_at, AlertRecord, Closing, ClosingKind, DueItem, HistoryEntry, HistoryWhat,
    Occurrence, Reconciliation, Reminder, SettingVersion, SignIn, SnoozeEnd, SnoozeRecord,
    SnoozeView, State, Undo, UpcomingItem, LAST_CHANCE_LEAD,
};
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
    #[error("the schedule can't be used: {0}")]
    BadSchedule(String),
    #[error("{0} is not a time zone this device knows")]
    BadZone(String),
    #[error("the countdown can't be used: {0}")]
    BadCountdown(String),
    #[error("{0} is not a countdown reminder")]
    NotCountdown(String),
    #[error("{0} has an open occurrence: complete or skip that instead")]
    StillOpen(String),
    #[error("{0} is not closed, or can't be undone or corrected")]
    NotClosed(String),
    #[error("a miss can't be undone; correct it to completed or skipped")]
    CantUndoMiss,
    #[error("that time hasn't come yet")]
    InTheFuture,
    #[error("a duration can't be negative")]
    BadDuration,
    #[error("the delay can't be used: {0}")]
    BadDelay(String),
    #[error("there is no reminder {0}")]
    NoReminder(String),
    #[error("that value was never one of the setting's")]
    NoSuchVersion,
    #[error("a snooze has to end in the future")]
    SnoozeInThePast,
    #[error("{0} is neither open nor expected, so it can't be snoozed")]
    NotExpected(String),
    #[error("there is no open occurrence {0}")]
    NotOpen(String),
    #[error("there is no list {0}")]
    NoList(String),
    #[error("the personal list can't be renamed or deleted")]
    PersonalList,
    #[error("the list's name is empty")]
    EmptyListName,
    #[error("{0} is not a colour (use #rrggbb)")]
    BadColour(String),
    #[error("the list still has reminders: move or delete them first")]
    ListNotEmpty,
    #[error("the reminder is already in that list")]
    SameList,
    #[error("a pause has to end in the future")]
    PauseInThePast,
    #[error("an event from the server could not be read: {0}")]
    BadEvent(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
