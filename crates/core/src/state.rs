use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::event::{Event, StoredEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub id: String,
    pub list_id: String,
    pub title: String,
    pub fire_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub id: String,
    pub reminder_id: String,
    pub scheduled_at: i64,
    pub fired_at: i64,
    /// `(who, when)` once completed.
    pub completed: Option<(String, i64)>,
}

/// An open occurrence for the Inbox's Due section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DueItem {
    pub occurrence_id: String,
    pub title: String,
    pub scheduled_at: i64,
    pub fired_at: i64,
    /// A change to it hasn't been received by the server yet.
    pub not_sent: bool,
}

/// A reminder that has not fired yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpcomingItem {
    pub reminder_id: String,
    pub title: String,
    pub fire_at: i64,
    /// A change to it hasn't been received by the server yet.
    pub not_sent: bool,
}

/// The current state, built by applying a stream's events in order.
#[derive(Debug, Default, Clone)]
pub struct State {
    pub reminders: BTreeMap<String, Reminder>,
    pub occurrences: BTreeMap<String, Occurrence>,
    /// Reminders and occurrences with a change the server hasn't numbered.
    unsent_reminders: BTreeSet<String>,
    unsent_occurrences: BTreeSet<String>,
}

impl State {
    /// Applies one event. Applying is forgiving so streams from several
    /// devices converge: the first opening of an occurrence and the first
    /// completion win, and later duplicates are ignored.
    pub fn apply(&mut self, stored: &StoredEvent) {
        if stored.seq.is_none() {
            match &stored.event {
                Event::ReminderCreated { reminder_id, .. } => {
                    self.unsent_reminders.insert(reminder_id.clone());
                }
                Event::OccurrenceOpened { occurrence_id, .. }
                | Event::OccurrenceCompleted { occurrence_id, .. } => {
                    self.unsent_occurrences.insert(occurrence_id.clone());
                }
            }
        }
        match &stored.event {
            Event::ReminderCreated {
                reminder_id,
                title,
                fire_at,
            } => {
                self.reminders
                    .entry(reminder_id.clone())
                    .or_insert_with(|| Reminder {
                        id: reminder_id.clone(),
                        list_id: stored.list_id.clone(),
                        title: title.clone(),
                        fire_at: *fire_at,
                    });
            }
            Event::OccurrenceOpened {
                occurrence_id,
                reminder_id,
                scheduled_at,
                fired_at,
            } => {
                self.occurrences
                    .entry(occurrence_id.clone())
                    .or_insert_with(|| Occurrence {
                        id: occurrence_id.clone(),
                        reminder_id: reminder_id.clone(),
                        scheduled_at: *scheduled_at,
                        fired_at: *fired_at,
                        completed: None,
                    });
            }
            Event::OccurrenceCompleted {
                occurrence_id,
                completed_at,
            } => {
                if let Some(o) = self.occurrences.get_mut(occurrence_id) {
                    if o.completed.is_none() {
                        o.completed = Some((stored.author.clone(), *completed_at));
                    }
                }
            }
        }
    }

    /// Whether the reminder has fired yet (its one occurrence exists).
    fn has_fired(&self, reminder_id: &str) -> bool {
        self.occurrences
            .values()
            .any(|o| o.reminder_id == reminder_id)
    }

    /// One-off reminders whose time has come and that have not fired.
    /// This covers reminders whose time passed while the app was closed.
    pub fn pending_firings(&self, now: i64) -> Vec<&Reminder> {
        let mut v: Vec<&Reminder> = self
            .reminders
            .values()
            .filter(|r| r.fire_at <= now && !self.has_fired(&r.id))
            .collect();
        v.sort_by_key(|r| (r.fire_at, r.id.clone()));
        v
    }

    /// The earliest time an unfired reminder is due.
    pub fn next_fire_at(&self) -> Option<i64> {
        self.reminders
            .values()
            .filter(|r| !self.has_fired(&r.id))
            .map(|r| r.fire_at)
            .min()
    }

    /// Open occurrences, oldest first. (Overdue arrives with priorities;
    /// for now every open occurrence is due.)
    pub fn due(&self) -> Vec<DueItem> {
        let mut v: Vec<DueItem> = self
            .occurrences
            .values()
            .filter(|o| o.completed.is_none())
            .filter_map(|o| {
                let r = self.reminders.get(&o.reminder_id)?;
                Some(DueItem {
                    occurrence_id: o.id.clone(),
                    title: r.title.clone(),
                    scheduled_at: o.scheduled_at,
                    fired_at: o.fired_at,
                    not_sent: self.unsent_occurrences.contains(&o.id)
                        || self.unsent_reminders.contains(&r.id),
                })
            })
            .collect();
        v.sort_by_key(|d| (d.fired_at, d.occurrence_id.clone()));
        v
    }

    pub fn upcoming(&self) -> Vec<UpcomingItem> {
        let mut v: Vec<UpcomingItem> = self
            .reminders
            .values()
            .filter(|r| !self.has_fired(&r.id))
            .map(|r| UpcomingItem {
                reminder_id: r.id.clone(),
                title: r.title.clone(),
                fire_at: r.fire_at,
                not_sent: self.unsent_reminders.contains(&r.id),
            })
            .collect();
        v.sort_by_key(|u| (u.fire_at, u.reminder_id.clone()));
        v
    }

    /// Whether any change hasn't been received by the server yet.
    pub fn has_unsent(&self) -> bool {
        !self.unsent_reminders.is_empty() || !self.unsent_occurrences.is_empty()
    }

    /// A one-off reminder is finished once its occurrence is closed.
    pub fn is_finished(&self, reminder_id: &str) -> bool {
        self.occurrences
            .values()
            .any(|o| o.reminder_id == reminder_id && o.completed.is_some())
    }
}
