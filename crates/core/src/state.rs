use crate::{Event, Millis};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reminder {
    pub id: String,
    pub title: String,
    pub due_at: Millis,
    /// A one-off is finished once its occurrence is closed.
    pub finished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceStatus {
    Due,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Occurrence {
    pub id: String,
    pub reminder_id: String,
    pub title: String,
    pub scheduled_at: Millis,
    pub fired_at: Millis,
    pub status: OccurrenceStatus,
    pub completed_by: Option<String>,
    pub completed_at: Option<Millis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxSection {
    Due,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InboxItem {
    pub section: InboxSection,
    pub occurrence: Occurrence,
}

/// The current state: the stream applied in order.
#[derive(Debug, Default)]
pub struct State {
    reminders: BTreeMap<String, Reminder>,
    occurrences: BTreeMap<String, Occurrence>,
    fired: HashSet<(String, Millis)>,
}

impl State {
    pub fn from_events(events: impl IntoIterator<Item = Event>) -> State {
        let mut state = State::default();
        for event in events {
            state.apply(&event);
        }
        state
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::ReminderCreated {
                reminder_id,
                title,
                due_at,
                ..
            } => {
                self.reminders.insert(
                    reminder_id.clone(),
                    Reminder {
                        id: reminder_id.clone(),
                        title: title.clone(),
                        due_at: *due_at,
                        finished: false,
                    },
                );
            }
            Event::OccurrenceFired {
                occurrence_id,
                reminder_id,
                scheduled_at,
                fired_at,
            } => {
                // Firings merge by identity: a second firing of the same one is a no-op.
                if self.occurrences.contains_key(occurrence_id) {
                    return;
                }
                let title = self
                    .reminders
                    .get(reminder_id)
                    .map(|r| r.title.clone())
                    .unwrap_or_default();
                self.fired.insert((reminder_id.clone(), *scheduled_at));
                self.occurrences.insert(
                    occurrence_id.clone(),
                    Occurrence {
                        id: occurrence_id.clone(),
                        reminder_id: reminder_id.clone(),
                        title,
                        scheduled_at: *scheduled_at,
                        fired_at: *fired_at,
                        status: OccurrenceStatus::Due,
                        completed_by: None,
                        completed_at: None,
                    },
                );
            }
            Event::OccurrenceCompleted {
                occurrence_id,
                by,
                at,
            } => {
                if let Some(o) = self.occurrences.get_mut(occurrence_id) {
                    if o.status == OccurrenceStatus::Due {
                        o.status = OccurrenceStatus::Completed;
                        o.completed_by = Some(by.clone());
                        o.completed_at = Some(*at);
                        if let Some(r) = self.reminders.get_mut(&o.reminder_id) {
                            r.finished = true;
                        }
                    }
                }
            }
        }
    }

    pub fn reminders(&self) -> impl Iterator<Item = &Reminder> {
        self.reminders.values()
    }

    pub fn occurrence(&self, id: &str) -> Option<&Occurrence> {
        self.occurrences.get(id)
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.occurrence(id)
            .is_some_and(|o| o.status == OccurrenceStatus::Due)
    }

    pub fn has_fired(&self, reminder_id: &str, scheduled_at: Millis) -> bool {
        self.fired
            .contains(&(reminder_id.to_string(), scheduled_at))
    }

    pub fn inbox(&self) -> Vec<InboxItem> {
        let mut items: Vec<_> = self
            .occurrences
            .values()
            .filter(|o| o.status == OccurrenceStatus::Due)
            .cloned()
            .map(|occurrence| InboxItem {
                section: InboxSection::Due,
                occurrence,
            })
            .collect();
        items.sort_by_key(|i| i.occurrence.scheduled_at);
        items
    }

    pub fn next_due(&self) -> Option<Millis> {
        self.reminders
            .values()
            .filter(|r| !r.finished && !self.has_fired(&r.id, r.due_at))
            .map(|r| r.due_at)
            .min()
    }
}
