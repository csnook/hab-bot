//! The reminder core: an event stream in SQLite, and the state built by applying it.
//!
//! Every change is an [`Event`] appended to the personal list's stream. The current
//! [`State`] is never stored; it is rebuilt by applying the stream (ADR 0005), so
//! sync can later merge streams from several devices.

mod schedule;
mod state;
mod store;
pub mod time;

pub use state::{InboxItem, InboxSection, Occurrence, OccurrenceStatus, Reminder, State, Trigger};
pub use store::Store;

use chrono_tz::Tz;
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
    #[error("bad schedule: {0}")]
    BadSchedule(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One recorded change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    ReminderCreated {
        reminder_id: String,
        title: String,
        #[serde(default)]
        triggers: Vec<Trigger>,
        /// A named time zone, or none for floating.
        #[serde(default)]
        tz: Option<String>,
        /// Only in events from the walking skeleton, before triggers existed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        due_at: Option<Millis>,
        created_at: Millis,
    },
    /// A reminder fired: an occurrence opened. The identity of an occurrence is the
    /// reminder plus its scheduled instance, so firings on several devices merge.
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
    /// The app closed an occurrence nobody dealt with, because a newer instance fired.
    OccurrenceMissed { occurrence_id: String, at: Millis },
}

/// What the UI sends to make a reminder.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NewReminder {
    pub title: String,
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub tz: Option<String>,
}

/// The identity of an occurrence: the reminder plus its instance's key.
pub fn occurrence_id(reminder_id: &str, key: &str) -> String {
    format!("{reminder_id}@{key}")
}

/// How far ahead expected occurrences are predicted when looking for the next wake-up.
const HORIZON: Millis = 400 * 24 * 3600 * 1000;

/// The core: a store plus the state built from it.
pub struct Core {
    store: Store,
    state: State,
    user: String,
    /// The zone floating reminders follow: the device's current one.
    zone: Tz,
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
            zone: Tz::UTC,
        })
    }

    /// Sets the device's time zone, which floating reminders follow. Call it whenever the
    /// device's zone may have changed. An unknown name is ignored.
    pub fn set_zone(&mut self, name: &str) {
        if let Ok(zone) = time::parse_zone(name) {
            self.zone = zone;
        }
    }

    /// Rebuilds the state from the stream. Another connection to the same database (the
    /// Android alarm path runs a second core in this process) may have appended events.
    pub fn refresh(&mut self) -> Result<()> {
        self.state = State::from_events(self.store.events(PERSONAL_LIST)?);
        Ok(())
    }

    fn record(&mut self, event: Event) -> Result<()> {
        self.store.append(PERSONAL_LIST, &event)?;
        self.state.apply(&event);
        Ok(())
    }

    /// Creates a one-off reminder in the personal list.
    pub fn create_one_off(&mut self, title: &str, due_at: Millis, now: Millis) -> Result<String> {
        self.create(
            NewReminder {
                title: title.into(),
                triggers: vec![Trigger::OneOff { at: due_at }],
                tz: None,
            },
            now,
        )
    }

    /// Creates a reminder in the personal list. Schedules and zones are checked first.
    pub fn create(&mut self, new: NewReminder, now: Millis) -> Result<String> {
        let title = new.title.trim();
        if title.is_empty() {
            return Err(Error::EmptyTitle);
        }
        if new.triggers.is_empty() {
            return Err(Error::BadSchedule("a reminder needs a trigger".into()));
        }
        if let Some(tz) = &new.tz {
            time::parse_zone(tz)?;
        }
        for trigger in &new.triggers {
            if let Trigger::Schedule { rule, start } = trigger {
                time::validate(rule, time::parse_wall(start)?)?;
            }
        }
        let reminder_id = uuid::Uuid::new_v4().to_string();
        self.record(Event::ReminderCreated {
            reminder_id: reminder_id.clone(),
            title: title.to_string(),
            triggers: new.triggers,
            tz: new.tz,
            due_at: None,
            created_at: now,
        })?;
        Ok(reminder_id)
    }

    /// Fires every reminder with an instance that has come due, including instances that
    /// passed while the device was off or asleep. Returns the occurrences left open.
    ///
    /// - A reminder has at most one open occurrence (ADR 0001): a new instance marks the
    ///   previous one missed.
    /// - Of several instances that passed unseen, only the latest is left open. The
    ///   earlier ones are recorded as missed.
    /// - Overdue time and expiry count from the scheduled time, not from now.
    pub fn fire_due(&mut self, now: Millis) -> Result<Vec<Occurrence>> {
        let mut opened = Vec::new();
        let ids: Vec<String> = self
            .state
            .reminders()
            .filter(|r| !r.finished)
            .map(|r| r.id.clone())
            .collect();
        for rid in ids {
            let reminder = self.state.reminder(&rid).expect("listed").clone();
            // A one-off scheduled in the past still fires once; its identity stops repeats.
            let after = if reminder.is_one_off() {
                Millis::MIN
            } else {
                self.state.evaluated_through(&reminder)
            };
            let due: Vec<_> = schedule::instances(&reminder, self.zone, after, now)
                .into_iter()
                .filter(|i| !self.state.has_fired(&rid, &occurrence_id(&rid, &i.key)))
                .collect();
            let count = due.len();
            for (n, instance) in due.into_iter().enumerate() {
                let id = occurrence_id(&rid, &instance.key);
                // The previous open occurrence is missed when the next instance fires.
                if let Some(previous) = self.state.open_occurrence(&rid).map(|o| o.id.clone()) {
                    self.record(Event::OccurrenceMissed {
                        occurrence_id: previous,
                        at: instance.scheduled_at,
                    })?;
                }
                let latest = n + 1 == count;
                self.record(Event::OccurrenceFired {
                    occurrence_id: id.clone(),
                    reminder_id: rid.clone(),
                    scheduled_at: instance.scheduled_at,
                    fired_at: if latest { now } else { instance.scheduled_at },
                })?;
                if latest {
                    opened.push(self.state.occurrence(&id).expect("just fired").clone());
                }
            }
        }
        // An earlier instance fired in this call is closed by the next one; a lone old
        // instance is left open and goes overdue as the priority says.
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

    /// Occurrences predicted to come due in `(now, to]`, soonest first.
    pub fn expected(&self, now: Millis, to: Millis) -> Vec<Occurrence> {
        let mut out = Vec::new();
        for r in self.state.reminders().filter(|r| !r.finished) {
            for i in schedule::instances(r, self.zone, now, to) {
                let id = occurrence_id(&r.id, &i.key);
                if self.state.has_fired(&r.id, &id) {
                    continue;
                }
                out.push(Occurrence {
                    id,
                    reminder_id: r.id.clone(),
                    title: r.title.clone(),
                    scheduled_at: i.scheduled_at,
                    fired_at: i.scheduled_at,
                    status: OccurrenceStatus::Expected,
                    completed_by: None,
                    closed_at: None,
                });
            }
        }
        out.sort_by_key(|o| o.scheduled_at);
        out
    }

    /// The Inbox: open occurrences under Due, what's expected for the rest of today under
    /// Later today, and what closed today (including missed) under Earlier today.
    pub fn inbox(&self, now: Millis) -> Vec<InboxItem> {
        let today = time::wall_at(now, self.zone).date();
        let start = time::resolve(today.and_hms_opt(0, 0, 0).expect("midnight"), self.zone);
        let end = time::resolve(
            (today + chrono::Duration::days(1))
                .and_hms_opt(0, 0, 0)
                .expect("midnight"),
            self.zone,
        );
        let item = |section, occurrence: Occurrence| InboxItem {
            section,
            occurrence,
        };

        let mut due: Vec<_> = self
            .state
            .occurrences()
            .filter(|o| o.status == OccurrenceStatus::Due)
            .cloned()
            .collect();
        due.sort_by_key(|o| o.scheduled_at);
        let mut earlier: Vec<_> = self
            .state
            .occurrences()
            .filter(|o| {
                matches!(
                    o.status,
                    OccurrenceStatus::Completed | OccurrenceStatus::Missed
                )
            })
            .filter(|o| o.closed_at.is_some_and(|c| c >= start && c < end))
            .cloned()
            .collect();
        earlier.sort_by_key(|o| o.closed_at);

        due.into_iter()
            .map(|o| item(InboxSection::Due, o))
            .chain(
                self.expected(now, end - 1)
                    .into_iter()
                    .map(|o| item(InboxSection::LaterToday, o)),
            )
            .chain(
                earlier
                    .into_iter()
                    .map(|o| item(InboxSection::EarlierToday, o)),
            )
            .collect()
    }

    /// The earliest instant at which a reminder has an instance not yet fired, for
    /// scheduling the next wake-up. It can be in the past: something is already due.
    pub fn next_due(&self, now: Millis) -> Option<Millis> {
        self.state
            .reminders()
            .filter(|r| !r.finished)
            .filter_map(|r| {
                let after = if r.is_one_off() {
                    Millis::MIN
                } else {
                    self.state.evaluated_through(r)
                };
                schedule::instances(r, self.zone, after, now.saturating_add(HORIZON))
                    .into_iter()
                    .find(|i| !self.state.has_fired(&r.id, &occurrence_id(&r.id, &i.key)))
                    .map(|i| i.scheduled_at)
            })
            .min()
    }

    pub fn state(&self) -> &State {
        &self.state
    }
}

#[cfg(test)]
mod tests;
