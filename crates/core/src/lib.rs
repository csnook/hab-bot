//! The reminder core: an event stream in SQLite, and the state built by applying it.
//!
//! Every change is an [`Event`] appended to the personal list's stream. The current
//! [`State`] is never stored; it is rebuilt by applying the stream (ADR 0005), so
//! sync can later merge streams from several devices.

mod alerts;
mod priority;
mod schedule;
mod state;
mod store;
pub mod time;

pub use alerts::{Alert, AlertEngine, AlertKind, Poll, LAST_CHANCE};
pub use priority::{all_settings, AlertStyle, Escalation, Priority, PrioritySettings};
pub use state::{
    CountdownUnit, InboxItem, InboxSection, Occurrence, OccurrenceStatus, Reminder, State, Trigger,
};
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
        #[serde(default)]
        priority: Priority,
        #[serde(default)]
        expiry: Option<Millis>,
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
        /// The recorded time of doing it. It can be edited, even to before the firing
        /// ("took it at 6:55").
        at: Millis,
        /// When it was actually tapped. The time the server received it joins this once
        /// sync exists.
        #[serde(default)]
        tapped_at: Option<Millis>,
    },
    /// Undo of a completion or skip. It reopens the occurrence, or, if it could no longer
    /// be open, closes it as missed at `missed_at`. The history keeps the original.
    OccurrenceUndone {
        occurrence_id: String,
        by: String,
        at: Millis,
        #[serde(default)]
        missed_at: Option<Millis>,
    },
    /// Any closed occurrence changed to completed or skipped, at a recorded time. The
    /// history keeps both the original and the correction.
    OccurrenceCorrected {
        occurrence_id: String,
        by: String,
        to: Outcome,
        /// The recorded time.
        at: Millis,
        corrected_at: Millis,
        #[serde(default)]
        note: Option<String>,
    },
    /// An alert changed style on a device: the first alert, or a step of escalation. History
    /// keeps these, not repeats.
    AlertChanged {
        occurrence_id: String,
        style: AlertStyle,
        device: String,
        at: Millis,
    },
    /// Closed by someone's choice, with an optional note.
    OccurrenceSkipped {
        occurrence_id: String,
        by: String,
        at: Millis,
        note: Option<String>,
    },
    /// Alerts for an occurrence are quiet until `until`. It may be set ahead of time, on an
    /// expected occurrence.
    Snoozed {
        occurrence_id: String,
        until: Millis,
        by: String,
        at: Millis,
        via: SnoozeVia,
    },
    /// A snooze ended: how it actually ended, next to what it was set to end on.
    SnoozeEnded {
        occurrence_id: String,
        at: Millis,
        how: SnoozeEnd,
    },
    PriorityChanged {
        reminder_id: String,
        priority: Priority,
    },
    /// The app closed an occurrence nobody dealt with, because a newer instance fired.
    OccurrenceMissed { occurrence_id: String, at: Millis },
}

/// What a correction changes an occurrence to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Skipped,
}

/// How a snooze was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnoozeVia {
    Button,
    /// Swiping a notification away (Android).
    Swipe,
}

/// How a snooze actually ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnoozeEnd {
    /// It ran out.
    Elapsed,
    /// Snoozed again, with no limit on repeats.
    Replaced,
    /// The occurrence closed.
    Closed,
}

/// A way to snooze, as the picker lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnoozeOption {
    pub label: String,
    pub until: Millis,
}

/// What the snooze picker shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnoozePicker {
    pub options: Vec<SnoozeOption>,
    /// "expires at 23:59": a known expiry that falls inside the longest options.
    pub expires_at: Option<Millis>,
}

/// What the UI sends to make a reminder.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NewReminder {
    pub title: String,
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub tz: Option<String>,
    #[serde(default)]
    pub priority: Priority,
    /// How long after each scheduled time its occurrence expires, if it does.
    #[serde(default)]
    pub expiry: Option<Millis>,
}

/// The identity of an occurrence: the reminder plus its instance's key.
pub fn occurrence_id(reminder_id: &str, key: &str) -> String {
    format!("{reminder_id}@{key}")
}

/// Minimum and Low occurrences overdue for more than 7 days fold into one row. Medium and
/// above never fold.
const FOLD_AFTER: Millis = 7 * 24 * 3600 * 1000;

fn is_old_quiet(o: &Occurrence, now: Millis) -> bool {
    o.priority <= Priority::Low && now >= o.overdue_at && now - o.overdue_at > FOLD_AFTER
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

    /// Changes a reminder's priority. Its overdue time follows, as it isn't overridden.
    pub fn set_priority(&mut self, reminder_id: &str, priority: Priority) -> Result<()> {
        if self.state.reminder(reminder_id).is_none() {
            return Err(Error::NoOpenOccurrence(reminder_id.to_string()));
        }
        self.record(Event::PriorityChanged {
            reminder_id: reminder_id.to_string(),
            priority,
        })
    }

    /// Rebuilds the state from the stream. Another connection to the same database (the
    /// Android alarm path runs a second core in this process) may have appended events.
    pub fn refresh(&mut self) -> Result<()> {
        self.state = State::from_events(self.store.events(PERSONAL_LIST)?);
        Ok(())
    }

    pub(crate) fn record(&mut self, event: Event) -> Result<()> {
        // Closing an occurrence ends its snooze.
        if let Event::OccurrenceCompleted {
            occurrence_id, at, ..
        }
        | Event::OccurrenceSkipped {
            occurrence_id, at, ..
        }
        | Event::OccurrenceMissed { occurrence_id, at } = &event
        {
            if self.state.snoozed_until(occurrence_id).is_some() {
                let ended = Event::SnoozeEnded {
                    occurrence_id: occurrence_id.clone(),
                    at: *at,
                    how: SnoozeEnd::Closed,
                };
                self.store.append(PERSONAL_LIST, &ended)?;
                self.state.apply(&ended);
            }
        }
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
                priority: Priority::Medium,
                expiry: None,
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
            match trigger {
                Trigger::Schedule { rule, start } => {
                    time::validate(rule, time::parse_wall(start)?)?
                }
                Trigger::Countdown {
                    amount, at, unit, ..
                } => {
                    if *amount == 0 {
                        return Err(Error::BadSchedule("a countdown needs an amount".into()));
                    }
                    if at.is_some() && *unit == CountdownUnit::Hours {
                        return Err(Error::BadSchedule(
                            "only day countdowns have a time of day".into(),
                        ));
                    }
                    if let Some(at) = at {
                        chrono::NaiveTime::parse_from_str(at, "%H:%M")
                            .map_err(|_| Error::BadSchedule(format!("bad time {at:?}")))?;
                    }
                    if new.triggers.len() > 1 {
                        return Err(Error::BadSchedule("a countdown stands alone".into()));
                    }
                }
                Trigger::OneOff { .. } => {}
            }
        }
        let reminder_id = uuid::Uuid::new_v4().to_string();
        self.record(Event::ReminderCreated {
            reminder_id: reminder_id.clone(),
            title: title.to_string(),
            triggers: new.triggers,
            tz: new.tz,
            priority: new.priority,
            expiry: new.expiry,
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
        self.end_elapsed_snoozes(now)?;
        self.expire_due(now)?;
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
            let after = self.window_start(&reminder);
            let due: Vec<_> = schedule::instances(&self.state, &reminder, self.zone, after, now)
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

    /// Opens an expected occurrence early and returns its id. Its scheduled time stays the
    /// identity, so it never fires again.
    fn fire_expected(&mut self, expected_id: &str, now: Millis) -> Result<String> {
        let o = self
            .expected(now, now.saturating_add(HORIZON))
            .into_iter()
            .find(|o| o.id == expected_id)
            .ok_or_else(|| Error::NoOpenOccurrence(expected_id.to_string()))?;
        self.record(Event::OccurrenceFired {
            occurrence_id: o.id.clone(),
            reminder_id: o.reminder_id,
            scheduled_at: o.scheduled_at,
            fired_at: now,
        })?;
        Ok(o.id)
    }

    /// The reminder's next expected occurrence, if it has one. Reminders with none (such as
    /// "when I arrive at the gym") don't offer early completion.
    pub fn next_expected(&self, reminder_id: &str, now: Millis) -> Option<Occurrence> {
        self.expected(now, now.saturating_add(HORIZON))
            .into_iter()
            .find(|o| o.reminder_id == reminder_id)
    }

    /// Completes a reminder ahead of its next expected occurrence, which then never fires.
    /// A countdown restarts from `at`, and a wait would end.
    pub fn complete_early(&mut self, reminder_id: &str, at: Millis, now: Millis) -> Result<()> {
        let next = self
            .next_expected(reminder_id, now)
            .ok_or_else(|| Error::NoOpenOccurrence(reminder_id.to_string()))?;
        let id = self.fire_expected(&next.id, now)?;
        self.complete_at(&id, at.min(now), now)
    }

    /// Skips an expected occurrence ahead of time: it never fires.
    pub fn skip_ahead(
        &mut self,
        expected_id: &str,
        note: Option<String>,
        now: Millis,
    ) -> Result<()> {
        let id = self.fire_expected(expected_id, now)?;
        self.skip(&id, note, now)
    }

    /// The notes used on recent skips, newest first and without repeats, to offer again.
    pub fn recent_skip_notes(&self, limit: usize) -> Vec<String> {
        let mut notes: Vec<String> = Vec::new();
        for e in self.history().into_iter().rev() {
            if let Event::OccurrenceSkipped { note: Some(n), .. }
            | Event::OccurrenceCorrected {
                note: Some(n),
                to: Outcome::Skipped,
                ..
            } = e
            {
                if !n.trim().is_empty() && !notes.contains(&n) {
                    notes.push(n);
                }
            }
            if notes.len() == limit {
                break;
            }
        }
        notes
    }

    /// Undoes a completion or skip. The occurrence reopens if it would still be open: not
    /// expired, and no newer occurrence of the reminder has come up. Otherwise it becomes
    /// missed, at the expiry or when the newer one came up, which also moves a countdown back.
    pub fn undo(&mut self, occurrence_id: &str, now: Millis) -> Result<()> {
        let o = self
            .state
            .occurrence(occurrence_id)
            .filter(|o| {
                matches!(
                    o.status,
                    OccurrenceStatus::Completed | OccurrenceStatus::Skipped
                )
            })
            .ok_or_else(|| Error::NoOpenOccurrence(occurrence_id.to_string()))?
            .clone();
        let expired = o.expires_at.filter(|e| *e <= now);
        let newer = self
            .state
            .occurrences()
            .filter(|n| n.reminder_id == o.reminder_id && n.id != o.id)
            .filter(|n| n.scheduled_at > o.scheduled_at || n.status == OccurrenceStatus::Due)
            .map(|n| n.scheduled_at.max(o.scheduled_at))
            .min();
        let missed_at = expired.or(newer);
        self.record(Event::OccurrenceUndone {
            occurrence_id: occurrence_id.to_string(),
            by: self.user.clone(),
            at: now,
            missed_at,
        })
    }

    /// Changes any closed occurrence, including a missed one, to completed or skipped at a
    /// time. A miss corrected to completed counts as done late.
    pub fn correct(
        &mut self,
        occurrence_id: &str,
        to: Outcome,
        recorded_at: Millis,
        note: Option<String>,
        now: Millis,
    ) -> Result<()> {
        let closed = self.state.occurrence(occurrence_id).is_some_and(|o| {
            matches!(
                o.status,
                OccurrenceStatus::Completed | OccurrenceStatus::Skipped | OccurrenceStatus::Missed
            )
        });
        if !closed {
            return Err(Error::NoOpenOccurrence(occurrence_id.to_string()));
        }
        if recorded_at > now {
            return Err(Error::BadSchedule(
                "a correction can't be recorded in the future".into(),
            ));
        }
        self.record(Event::OccurrenceCorrected {
            occurrence_id: occurrence_id.to_string(),
            by: self.user.clone(),
            to,
            at: recorded_at,
            corrected_at: now,
            note,
        })
    }

    /// Completes an open occurrence now, recording who and when. A one-off is then finished.
    pub fn complete(&mut self, occurrence_id: &str, now: Millis) -> Result<()> {
        self.complete_at(occurrence_id, now, now)
    }

    /// Completes an open occurrence at a time the user says, which can be before the firing
    /// ("took it at 6:55"). The completion keeps both the time said and the time tapped.
    pub fn complete_at(
        &mut self,
        occurrence_id: &str,
        recorded_at: Millis,
        now: Millis,
    ) -> Result<()> {
        if !self.state.is_open(occurrence_id) {
            return Err(Error::NoOpenOccurrence(occurrence_id.to_string()));
        }
        if recorded_at > now {
            return Err(Error::BadSchedule(
                "a completion can't be recorded in the future".into(),
            ));
        }
        self.record(Event::OccurrenceCompleted {
            occurrence_id: occurrence_id.to_string(),
            by: self.user.clone(),
            at: recorded_at,
            tapped_at: Some(now),
        })
    }

    /// Where evaluation resumes. One-offs and countdowns are tracked by identity instead,
    /// so a one-off scheduled in the past still fires once, and a countdown's next
    /// firing may come before the last one's scheduled time (completed early).
    fn window_start(&self, r: &Reminder) -> Millis {
        if r.is_one_off() || r.is_countdown() {
            Millis::MIN
        } else {
            self.state.evaluated_through(r)
        }
    }

    /// Occurrences predicted to come due in `(now, to]`, soonest first.
    pub fn expected(&self, now: Millis, to: Millis) -> Vec<Occurrence> {
        let mut out = Vec::new();
        for r in self.state.reminders().filter(|r| !r.finished) {
            for i in schedule::instances(&self.state, r, self.zone, now, to) {
                let id = occurrence_id(&r.id, &i.key);
                if self.state.has_fired(&r.id, &id) {
                    continue;
                }
                let snoozed_until = self.state.snoozed_until(&id);
                out.push(Occurrence {
                    id,
                    reminder_id: r.id.clone(),
                    title: r.title.clone(),
                    scheduled_at: i.scheduled_at,
                    fired_at: i.scheduled_at,
                    priority: r.priority,
                    overdue_at: r.priority.overdue_at(i.scheduled_at),
                    snoozed_until,
                    expires_at: r.expiry.map(|e| i.scheduled_at + e),
                    status: OccurrenceStatus::Expected,
                    completed_by: None,
                    closed_at: None,
                    tapped_at: None,
                    note: None,
                    corrected_from: None,
                });
            }
        }
        out.sort_by_key(|o| o.scheduled_at);
        out
    }

    /// Open occurrences as they stand now: a reminder's priority can change, which moves
    /// its overdue time unless overridden.
    pub fn open_occurrences(&self) -> Vec<Occurrence> {
        self.state
            .occurrences()
            .filter(|o| o.status == OccurrenceStatus::Due)
            .map(|o| {
                let mut o = o.clone();
                if let Some(r) = self.state.reminder(&o.reminder_id) {
                    o.priority = r.priority;
                    o.overdue_at = r.priority.overdue_at(o.scheduled_at);
                }
                o
            })
            .collect()
    }

    /// Records that an alert on `device` changed style.
    pub(crate) fn record_alert(
        &mut self,
        occurrence_id: &str,
        style: AlertStyle,
        device: &str,
        at: Millis,
    ) -> Result<()> {
        self.record(Event::AlertChanged {
            occurrence_id: occurrence_id.into(),
            style,
            device: device.into(),
            at,
        })
    }

    /// The snooze one tap uses: the priority's current interval (1 day for Minimum and Low;
    /// Medium 1 hour while due and 10 minutes once overdue; High and Maximum 10 minutes).
    pub fn default_snooze_until(&self, occurrence_id: &str, now: Millis) -> Option<Millis> {
        let o = self.open_or_expected(occurrence_id, now)?;
        let s = o.priority.settings();
        Some(
            now + if now < o.overdue_at {
                s.due_interval
            } else {
                s.overdue_interval
            },
        )
    }

    fn open_or_expected(&self, id: &str, now: Millis) -> Option<Occurrence> {
        self.open_occurrences()
            .into_iter()
            .find(|o| o.id == id)
            .or_else(|| {
                self.expected(now, now.saturating_add(HORIZON))
                    .into_iter()
                    .find(|o| o.id == id)
            })
    }

    /// What the snooze menu offers: the priority's interval, 1 hour and tomorrow morning
    /// ("until a time" and "pick a time" are the UI's own pickers), and a known expiry.
    pub fn snooze_picker(&self, occurrence_id: &str, now: Millis) -> Option<SnoozePicker> {
        let o = self.open_or_expected(occurrence_id, now)?;
        let interval = self.default_snooze_until(occurrence_id, now)?;
        let tomorrow = time::wall_at(now, self.zone).date() + chrono::Duration::days(1);
        let morning = time::resolve(tomorrow.and_hms_opt(8, 0, 0).expect("8:00"), self.zone);
        let mut options = vec![SnoozeOption {
            label: "Default".into(),
            until: interval,
        }];
        for (label, until) in [("1 hour", now + 3_600_000), ("Tomorrow morning", morning)] {
            if options.iter().all(|opt| opt.until != until) {
                options.push(SnoozeOption {
                    label: label.into(),
                    until,
                });
            }
        }
        Some(SnoozePicker {
            options,
            expires_at: o.expires_at,
        })
    }

    /// Snoozes an open or expected occurrence until `until`. An expected one still fires at
    /// its time, quietly, and alerts when the snooze ends; its overdue time and expiry still
    /// count from the scheduled time. There's no limit on repeated snoozes.
    pub fn snooze(
        &mut self,
        occurrence_id: &str,
        until: Millis,
        via: SnoozeVia,
        now: Millis,
    ) -> Result<()> {
        if self.open_or_expected(occurrence_id, now).is_none() {
            return Err(Error::NoOpenOccurrence(occurrence_id.to_string()));
        }
        if until <= now {
            return Err(Error::BadSchedule("a snooze must end in the future".into()));
        }
        if self.state.snoozed_until(occurrence_id).is_some() {
            self.record(Event::SnoozeEnded {
                occurrence_id: occurrence_id.into(),
                at: now,
                how: SnoozeEnd::Replaced,
            })?;
        }
        self.record(Event::Snoozed {
            occurrence_id: occurrence_id.into(),
            until,
            by: self.user.clone(),
            at: now,
            via,
        })
    }

    /// One tap on Snooze. Returns when it ends.
    pub fn snooze_default(
        &mut self,
        occurrence_id: &str,
        via: SnoozeVia,
        now: Millis,
    ) -> Result<Millis> {
        let until = self
            .default_snooze_until(occurrence_id, now)
            .ok_or_else(|| Error::NoOpenOccurrence(occurrence_id.to_string()))?;
        self.snooze(occurrence_id, until, via, now)?;
        Ok(until)
    }

    /// Records the end of snoozes that ran out.
    fn end_elapsed_snoozes(&mut self, now: Millis) -> Result<()> {
        let elapsed: Vec<(String, Millis)> = self
            .state
            .snoozes()
            .filter(|(_, until)| **until <= now)
            .map(|(id, until)| (id.clone(), *until))
            .collect();
        for (occurrence_id, until) in elapsed {
            self.record(Event::SnoozeEnded {
                occurrence_id,
                at: until,
                how: SnoozeEnd::Elapsed,
            })?;
        }
        Ok(())
    }

    /// Closes occurrences whose expiry has passed as missed, at the expiry.
    fn expire_due(&mut self, now: Millis) -> Result<()> {
        let expired: Vec<(String, Millis)> = self
            .open_occurrences()
            .into_iter()
            .filter_map(|o| o.expires_at.filter(|e| *e <= now).map(|e| (o.id, e)))
            .collect();
        for (occurrence_id, at) in expired {
            self.record(Event::OccurrenceMissed { occurrence_id, at })?;
        }
        Ok(())
    }

    /// "Skip all…" on the folded row: skips every older quiet occurrence. Returns how many,
    /// which the dialog says first.
    pub fn skip_older_quiet(&mut self, now: Millis) -> Result<usize> {
        let ids: Vec<String> = self
            .open_occurrences()
            .into_iter()
            .filter(|o| is_old_quiet(o, now))
            .map(|o| o.id)
            .collect();
        for id in &ids {
            self.skip(id, None, now)?;
        }
        Ok(ids.len())
    }

    /// How many occurrences are folded, for the dialog.
    pub fn older_quiet_count(&self, now: Millis) -> usize {
        self.open_occurrences()
            .iter()
            .filter(|o| is_old_quiet(o, now))
            .count()
    }

    /// Skips an open occurrence: closed without doing it, by choice.
    pub fn skip(&mut self, occurrence_id: &str, note: Option<String>, now: Millis) -> Result<()> {
        if !self.state.is_open(occurrence_id) {
            return Err(Error::NoOpenOccurrence(occurrence_id.to_string()));
        }
        self.record(Event::OccurrenceSkipped {
            occurrence_id: occurrence_id.into(),
            by: self.user.clone(),
            at: now,
            note,
        })
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

        let (mut overdue, mut due): (Vec<_>, Vec<_>) = self
            .open_occurrences()
            .into_iter()
            .partition(|o| now >= o.overdue_at);
        // Overdue: highest priority first, then longest overdue.
        overdue.sort_by_key(|o| (std::cmp::Reverse(o.priority), o.overdue_at));
        // Old quiet occurrences fold away; nothing expires them by default.
        let (folded, overdue): (Vec<_>, Vec<_>) =
            overdue.into_iter().partition(|o| is_old_quiet(o, now));
        due.sort_by_key(|o| o.scheduled_at);
        let mut earlier: Vec<_> = self
            .state
            .occurrences()
            .filter(|o| {
                matches!(
                    o.status,
                    OccurrenceStatus::Completed
                        | OccurrenceStatus::Skipped
                        | OccurrenceStatus::Missed
                )
            })
            .filter(|o| o.closed_at.is_some_and(|c| c >= start && c < end))
            .cloned()
            .collect();
        earlier.sort_by_key(|o| o.closed_at);

        overdue
            .into_iter()
            .map(|o| item(InboxSection::Overdue, o))
            .chain(
                folded
                    .into_iter()
                    .map(|o| item(InboxSection::OverdueFolded, o)),
            )
            .chain(due.into_iter().map(|o| item(InboxSection::Due, o)))
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
                let after = self.window_start(r);
                schedule::instances(
                    &self.state,
                    r,
                    self.zone,
                    after,
                    now.saturating_add(HORIZON),
                )
                .into_iter()
                .find(|i| !self.state.has_fired(&r.id, &occurrence_id(&r.id, &i.key)))
                .map(|i| i.scheduled_at)
            })
            .min()
    }

    /// Every event in the personal list's stream, oldest first: the history.
    pub fn history(&self) -> Vec<Event> {
        self.store.events(PERSONAL_LIST).unwrap_or_default()
    }

    pub fn state(&self) -> &State {
        &self.state
    }
}

#[cfg(test)]
mod tests;
