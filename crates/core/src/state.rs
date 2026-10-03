use crate::{Event, Millis, Outcome, Priority};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// What prompts a reminder to fire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Once, at an instant.
    OneOff { at: Millis },
    /// An iCalendar recurrence rule (RRULE), starting at a wall-clock time such as
    /// `2026-10-05T07:00`. Several times a day are several triggers.
    Schedule { rule: String, start: String },
    /// A set time after the reminder's last occurrence closed.
    Countdown {
        /// `Hours` count elapsed time, whatever the zone. `Days` are calendar days and,
        /// with `at`, fire at a time of day in the reminder's zone ("3 days later, 9:00").
        unit: CountdownUnit,
        amount: u32,
        #[serde(default)]
        at: Option<String>,
        /// When it was last done, as asked on creation. `None` ("never") fires at once.
        #[serde(default)]
        last_done: Option<Millis>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountdownUnit {
    Hours,
    Days,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reminder {
    pub id: String,
    pub title: String,
    pub triggers: Vec<Trigger>,
    /// A named time zone, or `None` for floating: the clock wherever the device is.
    pub tz: Option<String>,
    pub priority: Priority,
    /// How long after an instance's scheduled time its occurrence expires, if it does.
    pub expiry: Option<Millis>,
    pub created_at: Millis,
    /// A one-off is finished once its occurrence is closed.
    pub finished: bool,
}

impl Reminder {
    pub fn is_one_off(&self) -> bool {
        self.triggers
            .iter()
            .all(|t| matches!(t, Trigger::OneOff { .. }))
    }

    /// Whether this is a countdown: its instances depend on when things closed, so fired
    /// instances are tracked by identity rather than by scheduled time.
    pub fn is_countdown(&self) -> bool {
        self.triggers
            .iter()
            .any(|t| matches!(t, Trigger::Countdown { .. }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceStatus {
    /// Predicted to come due; becomes an occurrence only if the reminder fires.
    Expected,
    Due,
    Completed,
    /// Closed by someone's choice, without doing it.
    Skipped,
    /// Closed by the app: a newer instance fired, or it expired. Nobody chose it.
    Missed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Occurrence {
    pub id: String,
    pub reminder_id: String,
    pub title: String,
    pub scheduled_at: Millis,
    pub fired_at: Millis,
    pub priority: Priority,
    /// When it goes (or went) overdue: the priority's due interval after the scheduled time.
    pub overdue_at: Millis,
    /// Alerts are quiet until then. The occurrence stays open and still goes overdue.
    pub snoozed_until: Option<Millis>,
    /// When it expires, if the reminder has an expiry at a known time: its duration after
    /// the scheduled time. Reaching it closes the occurrence as missed.
    pub expires_at: Option<Millis>,
    pub status: OccurrenceStatus,
    pub completed_by: Option<String>,
    /// When it was closed: the recorded time for a completion, the time it was
    /// superseded for a miss.
    pub closed_at: Option<Millis>,
    /// When a completion was tapped, if different from the recorded time.
    pub tapped_at: Option<Millis>,
    pub note: Option<String>,
    /// The status before a correction. A miss corrected to completed counts as done late.
    pub corrected_from: Option<OccurrenceStatus>,
}

impl Occurrence {
    pub fn done_late(&self) -> bool {
        self.status == OccurrenceStatus::Completed
            && self.corrected_from == Some(OccurrenceStatus::Missed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxSection {
    Overdue,
    /// Minimum and Low occurrences overdue for over a week, folded into one row at the end
    /// of the Overdue section.
    OverdueFolded,
    Due,
    LaterToday,
    EarlierToday,
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
    fired: HashSet<(String, String)>,
    /// Snoozes by occurrence, which may be set before it fires (snoozing ahead of time).
    snoozes: BTreeMap<String, Millis>,
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
                triggers,
                tz,
                priority,
                expiry,
                due_at,
                created_at,
            } => {
                // The walking skeleton recorded a bare `due_at`.
                let mut triggers = triggers.clone();
                if triggers.is_empty() {
                    triggers.extend(due_at.map(|at| Trigger::OneOff { at }));
                }
                self.reminders.insert(
                    reminder_id.clone(),
                    Reminder {
                        id: reminder_id.clone(),
                        title: title.clone(),
                        triggers,
                        tz: tz.clone(),
                        priority: *priority,
                        expiry: *expiry,
                        created_at: *created_at,
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
                let (title, priority, expiry) = self
                    .reminders
                    .get(reminder_id)
                    .map(|r| (r.title.clone(), r.priority, r.expiry))
                    .unwrap_or_default();
                self.fired
                    .insert((reminder_id.clone(), occurrence_id.clone()));
                self.occurrences.insert(
                    occurrence_id.clone(),
                    Occurrence {
                        id: occurrence_id.clone(),
                        reminder_id: reminder_id.clone(),
                        title,
                        scheduled_at: *scheduled_at,
                        fired_at: *fired_at,
                        priority,
                        overdue_at: priority.overdue_at(*scheduled_at),
                        snoozed_until: self.snoozes.get(occurrence_id).copied(),
                        expires_at: expiry.map(|e| scheduled_at + e),
                        status: OccurrenceStatus::Due,
                        completed_by: None,
                        closed_at: None,
                        tapped_at: None,
                        note: None,
                        corrected_from: None,
                    },
                );
            }
            Event::Snoozed {
                occurrence_id,
                until,
                ..
            } => {
                self.snoozes.insert(occurrence_id.clone(), *until);
                if let Some(o) = self.occurrences.get_mut(occurrence_id) {
                    o.snoozed_until = Some(*until);
                }
            }
            Event::SnoozeEnded { occurrence_id, .. } => {
                self.snoozes.remove(occurrence_id);
                if let Some(o) = self.occurrences.get_mut(occurrence_id) {
                    o.snoozed_until = None;
                }
            }
            Event::AlertChanged { .. } => {} // history only; no effect on the state
            Event::OccurrenceSkipped {
                occurrence_id,
                by,
                at,
                note,
            } => {
                self.close(
                    occurrence_id,
                    OccurrenceStatus::Skipped,
                    *at,
                    Some(by.clone()),
                );
                if let Some(o) = self.occurrences.get_mut(occurrence_id) {
                    o.note = note.clone();
                }
            }
            Event::PriorityChanged {
                reminder_id,
                priority,
            } => {
                if let Some(r) = self.reminders.get_mut(reminder_id) {
                    r.priority = *priority;
                }
            }
            Event::OccurrenceCompleted {
                occurrence_id,
                by,
                at,
                tapped_at,
            } => {
                self.close(
                    occurrence_id,
                    OccurrenceStatus::Completed,
                    *at,
                    Some(by.clone()),
                );
                if let Some(o) = self.occurrences.get_mut(occurrence_id) {
                    o.tapped_at = *tapped_at;
                }
            }
            Event::OccurrenceUndone {
                occurrence_id,
                missed_at,
                ..
            } => {
                let Some(o) = self.occurrences.get_mut(occurrence_id) else {
                    return;
                };
                if !matches!(
                    o.status,
                    OccurrenceStatus::Completed | OccurrenceStatus::Skipped
                ) {
                    return;
                }
                o.completed_by = None;
                o.tapped_at = None;
                o.note = None;
                match missed_at {
                    Some(at) => {
                        o.status = OccurrenceStatus::Missed;
                        o.closed_at = Some(*at);
                    }
                    None => {
                        o.status = OccurrenceStatus::Due;
                        o.closed_at = None;
                        if let Some(r) = self.reminders.get_mut(&o.reminder_id) {
                            r.finished = false;
                        }
                    }
                }
            }
            Event::OccurrenceCorrected {
                occurrence_id,
                by,
                to,
                at,
                note,
                ..
            } => {
                let Some(o) = self.occurrences.get_mut(occurrence_id) else {
                    return;
                };
                if o.status == OccurrenceStatus::Due || o.status == OccurrenceStatus::Expected {
                    return;
                }
                o.corrected_from.get_or_insert(o.status);
                o.status = match to {
                    Outcome::Completed => OccurrenceStatus::Completed,
                    Outcome::Skipped => OccurrenceStatus::Skipped,
                };
                o.closed_at = Some(*at);
                o.completed_by = Some(by.clone());
                o.note = note.clone();
                o.tapped_at = None;
            }
            Event::OccurrenceMissed { occurrence_id, at } => {
                self.close(occurrence_id, OccurrenceStatus::Missed, *at, None);
            }
        }
    }

    fn close(&mut self, id: &str, status: OccurrenceStatus, at: Millis, by: Option<String>) {
        let Some(o) = self.occurrences.get_mut(id) else {
            return;
        };
        if o.status != OccurrenceStatus::Due {
            return;
        }
        o.status = status;
        o.closed_at = Some(at);
        o.completed_by = by;
        if let Some(r) = self.reminders.get_mut(&o.reminder_id) {
            if r.is_one_off() {
                r.finished = true;
            }
        }
    }

    pub fn reminders(&self) -> impl Iterator<Item = &Reminder> {
        self.reminders.values()
    }

    pub fn reminder(&self, id: &str) -> Option<&Reminder> {
        self.reminders.get(id)
    }

    pub fn occurrence(&self, id: &str) -> Option<&Occurrence> {
        self.occurrences.get(id)
    }

    pub fn occurrences(&self) -> impl Iterator<Item = &Occurrence> {
        self.occurrences.values()
    }

    pub fn snoozes(&self) -> impl Iterator<Item = (&String, &Millis)> {
        self.snoozes.iter()
    }

    pub fn snoozed_until(&self, occurrence_id: &str) -> Option<Millis> {
        self.snoozes.get(occurrence_id).copied()
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.occurrence(id)
            .is_some_and(|o| o.status == OccurrenceStatus::Due)
    }

    /// The reminder's open occurrence, if any. There is at most one (ADR 0001).
    pub fn open_occurrence(&self, reminder_id: &str) -> Option<&Occurrence> {
        self.occurrences
            .values()
            .find(|o| o.reminder_id == reminder_id && o.status == OccurrenceStatus::Due)
    }

    pub fn has_fired(&self, reminder_id: &str, occurrence_id: &str) -> bool {
        self.fired
            .contains(&(reminder_id.to_string(), occurrence_id.to_string()))
    }

    /// When a countdown restarts from: the latest closing among the reminder's occurrences.
    /// A completion counts from its recorded time, a skip or a miss from when it closed.
    pub fn last_closed(&self, reminder_id: &str) -> Option<Millis> {
        self.occurrences
            .values()
            .filter(|o| o.reminder_id == reminder_id)
            .filter_map(|o| o.closed_at)
            .max()
    }

    /// The latest scheduled time of any of the reminder's occurrences, or its creation:
    /// instances after this have not been evaluated yet.
    pub fn evaluated_through(&self, r: &Reminder) -> Millis {
        self.occurrences
            .values()
            .filter(|o| o.reminder_id == r.id)
            .map(|o| o.scheduled_at)
            .max()
            .map_or(r.created_at, |s| s.max(r.created_at))
    }
}
