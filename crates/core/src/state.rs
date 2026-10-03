use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::event::{Change, Event, Setting, StoredEvent};
use crate::hlc::Hlc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub id: String,
    pub list_id: String,
    pub title: String,
    pub fire_at: i64,
    pub note: String,
}

/// How an occurrence was closed. The order is the order of the rules for
/// devices that disagree: a completion beats a skip or a miss, and a skip
/// stands over a miss, which nobody chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosingKind {
    Missed,
    Skipped,
    Completed,
}

/// What closed an occurrence, by whom and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closing {
    pub kind: ClosingKind,
    /// The user who did it.
    pub by: String,
    /// The device it was done on, as it appears on its events.
    pub device_id: String,
    pub at: i64,
    pub note: Option<String>,
    pub event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub id: String,
    pub reminder_id: String,
    pub scheduled_at: i64,
    pub fired_at: i64,
    pub closing: Option<Closing>,
    /// Alerts are quiet until then. Closing the occurrence ends a snooze.
    pub snoozed_until: Option<i64>,
    /// The current alert was silenced. Closing the occurrence ends it.
    pub acknowledged: bool,
}

impl Occurrence {
    /// `(who, when)` once completed.
    pub fn completed(&self) -> Option<(String, i64)> {
        self.closing
            .as_ref()
            .filter(|c| c.kind == ClosingKind::Completed)
            .map(|c| (c.by.clone(), c.at))
    }

    pub fn is_open(&self) -> bool {
        self.closing.is_none()
    }
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
    pub snoozed_until: Option<i64>,
    pub acknowledged: bool,
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

/// A device that signed in to the account, as announced in the personal list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    pub event_id: String,
    /// The server's id for the device, as it appears on its events.
    pub device_id: String,
    pub name: String,
    pub at: i64,
}

/// One value a setting has had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingVersion {
    pub event_id: String,
    pub hlc: Hlc,
    pub change: Change,
    pub by: String,
    pub device_id: String,
    pub recorded_at: i64,
    /// This is the value the setting has now. The others lost, and can be
    /// restored.
    pub current: bool,
}

/// Two devices closed the same occurrence in different ways without syncing,
/// and a completion beat the other. The device that lost is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    /// The losing event, which names the notice.
    pub id: String,
    pub occurrence_id: String,
    /// The device whose action lost.
    pub device_id: String,
    pub by: String,
    pub lost: ClosingKind,
    pub lost_at: i64,
}

/// The current state, built by applying a stream's events in order.
#[derive(Debug, Default, Clone)]
pub struct State {
    pub reminders: BTreeMap<String, Reminder>,
    pub occurrences: BTreeMap<String, Occurrence>,
    /// What each device (by id) calls itself.
    pub device_names: BTreeMap<String, String>,
    /// Devices that signed in, in stream order.
    pub sign_ins: Vec<SignIn>,
    /// Devices (by id) that were removed from the account.
    pub removed_devices: BTreeSet<String>,
    /// Closings that lost to a completion on another device, in stream order.
    pub reconciliations: Vec<Reconciliation>,
    /// Every value each reminder setting has had.
    versions: BTreeMap<(String, Setting), Vec<SettingVersion>>,
    /// Occurrences opened for a reminder that already had one (it fired on
    /// two devices with different scheduled times, because the time was
    /// edited in between), mapped to the one that counts.
    aliases: BTreeMap<String, String>,
    /// The latest clock any change has carried.
    latest_hlc: Hlc,
    /// Reminders and occurrences with a change the server hasn't numbered.
    unsent_reminders: BTreeSet<String>,
    unsent_occurrences: BTreeSet<String>,
}

impl State {
    /// Applies one event. Applying is forgiving so streams from several
    /// devices converge: duplicates and out-of-order events are ignored, the
    /// first opening of an occurrence and the first closing of each kind win,
    /// and the rules for devices that disagree decide the rest.
    pub fn apply(&mut self, stored: &StoredEvent) {
        if stored.seq.is_none() {
            match &stored.event {
                Event::ReminderCreated { reminder_id, .. }
                | Event::ReminderEdited { reminder_id, .. } => {
                    self.unsent_reminders.insert(reminder_id.clone());
                }
                Event::OccurrenceOpened { occurrence_id, .. }
                | Event::OccurrenceCompleted { occurrence_id, .. }
                | Event::OccurrenceSkipped { occurrence_id, .. }
                | Event::OccurrenceMissed { occurrence_id, .. }
                | Event::OccurrenceSnoozed { occurrence_id, .. }
                | Event::OccurrenceAcknowledged { occurrence_id } => {
                    let id = self.resolve(occurrence_id).to_string();
                    self.unsent_occurrences.insert(id);
                }
                Event::DeviceNamed { .. }
                | Event::DeviceSignedIn { .. }
                | Event::DeviceRemoved { .. } => {}
            }
        }
        match &stored.event {
            Event::ReminderCreated {
                reminder_id,
                title,
                fire_at,
            } => {
                if !self.reminders.contains_key(reminder_id) {
                    // What a reminder was created with is its first value,
                    // older than any edit.
                    for change in [Change::Title(title.clone()), Change::FireAt(*fire_at)] {
                        self.add_version(reminder_id, Hlc::default(), change, stored);
                    }
                    self.reminders.insert(
                        reminder_id.clone(),
                        Reminder {
                            id: reminder_id.clone(),
                            list_id: stored.list_id.clone(),
                            title: title.clone(),
                            fire_at: *fire_at,
                            note: String::new(),
                        },
                    );
                    self.refresh(reminder_id);
                }
            }
            Event::ReminderEdited {
                reminder_id,
                hlc,
                change,
            } => {
                let hlc = hlc.clamped(stored.recorded_at);
                self.add_version(reminder_id, hlc, change.clone(), stored);
                self.refresh(reminder_id);
            }
            Event::OccurrenceOpened {
                occurrence_id,
                reminder_id,
                scheduled_at,
                fired_at,
            } => {
                if self.occurrences.contains_key(occurrence_id)
                    || self.aliases.contains_key(occurrence_id)
                {
                    return;
                }
                // A one-off reminder has one occurrence (ADR 0001).
                let existing = self
                    .occurrences
                    .values()
                    .find(|o| &o.reminder_id == reminder_id)
                    .map(|o| o.id.clone());
                if let Some(existing) = existing {
                    self.aliases.insert(occurrence_id.clone(), existing);
                    return;
                }
                self.occurrences.insert(
                    occurrence_id.clone(),
                    Occurrence {
                        id: occurrence_id.clone(),
                        reminder_id: reminder_id.clone(),
                        scheduled_at: *scheduled_at,
                        fired_at: *fired_at,
                        closing: None,
                        snoozed_until: None,
                        acknowledged: false,
                    },
                );
            }
            Event::OccurrenceCompleted {
                occurrence_id,
                completed_at,
            } => self.close(
                occurrence_id,
                ClosingKind::Completed,
                *completed_at,
                None,
                stored,
            ),
            Event::OccurrenceSkipped {
                occurrence_id,
                skipped_at,
                note,
            } => self.close(
                occurrence_id,
                ClosingKind::Skipped,
                *skipped_at,
                note.clone(),
                stored,
            ),
            Event::OccurrenceMissed {
                occurrence_id,
                missed_at,
            } => self.close(occurrence_id, ClosingKind::Missed, *missed_at, None, stored),
            Event::OccurrenceSnoozed {
                occurrence_id,
                until,
            } => {
                // Closing beats snoozing, whichever arrives first.
                if let Some(o) = self.open_occurrence_mut(occurrence_id) {
                    o.snoozed_until = Some(*until);
                }
            }
            Event::OccurrenceAcknowledged { occurrence_id } => {
                if let Some(o) = self.open_occurrence_mut(occurrence_id) {
                    o.acknowledged = true;
                }
            }
            Event::DeviceNamed { name } => {
                self.device_names
                    .insert(stored.device_id.clone(), name.clone());
            }
            Event::DeviceRemoved { device_id } => {
                self.removed_devices.insert(device_id.clone());
            }
            Event::DeviceSignedIn { name } => {
                self.device_names
                    .insert(stored.device_id.clone(), name.clone());
                self.sign_ins.push(SignIn {
                    event_id: stored.event_id.clone(),
                    device_id: stored.device_id.clone(),
                    name: name.clone(),
                    at: stored.recorded_at,
                });
            }
        }
    }

    /// The occurrence an id stands for, which is another's if it merged.
    pub fn resolve<'a>(&'a self, occurrence_id: &'a str) -> &'a str {
        self.aliases
            .get(occurrence_id)
            .map_or(occurrence_id, String::as_str)
    }

    fn open_occurrence_mut(&mut self, occurrence_id: &str) -> Option<&mut Occurrence> {
        let id = self.resolve(occurrence_id).to_string();
        self.occurrences.get_mut(&id).filter(|o| o.is_open())
    }

    /// Closes an occurrence. If it is already closed, the rules for devices
    /// that disagree decide which closing counts: the same kind merges and the
    /// first counts; otherwise the higher kind does (completed over skipped
    /// over missed). When a completion beats another closing, that is
    /// recorded, for the device that lost to be told.
    fn close(
        &mut self,
        occurrence_id: &str,
        kind: ClosingKind,
        at: i64,
        note: Option<String>,
        stored: &StoredEvent,
    ) {
        let id = self.resolve(occurrence_id).to_string();
        let Some(o) = self.occurrences.get_mut(&id) else {
            return;
        };
        let new = Closing {
            kind,
            by: stored.author.clone(),
            device_id: stored.device_id.clone(),
            at,
            note,
            event_id: stored.event_id.clone(),
        };
        let (winner, loser) = match o.closing.take() {
            None => {
                o.snoozed_until = None;
                o.acknowledged = false;
                o.closing = Some(new);
                return;
            }
            Some(old) if old.kind == new.kind => {
                o.closing = Some(old);
                return;
            }
            Some(old) if old.kind > new.kind => (old, new),
            Some(old) => (new, old),
        };
        if winner.kind == ClosingKind::Completed {
            self.reconciliations.push(Reconciliation {
                id: loser.event_id.clone(),
                occurrence_id: id.clone(),
                device_id: loser.device_id.clone(),
                by: loser.by.clone(),
                lost: loser.kind,
                lost_at: loser.at,
            });
        }
        if let Some(o) = self.occurrences.get_mut(&id) {
            o.closing = Some(winner);
        }
    }

    fn add_version(&mut self, reminder_id: &str, hlc: Hlc, change: Change, stored: &StoredEvent) {
        let versions = self
            .versions
            .entry((reminder_id.to_string(), change.setting()))
            .or_default();
        if versions
            .iter()
            .any(|v| v.event_id == stored.event_id && v.hlc == hlc)
        {
            return;
        }
        if hlc > self.latest_hlc {
            self.latest_hlc = hlc.clone();
        }
        versions.push(SettingVersion {
            event_id: stored.event_id.clone(),
            hlc,
            change,
            by: stored.author.clone(),
            device_id: stored.device_id.clone(),
            recorded_at: stored.recorded_at,
            current: false,
        });
    }

    /// Sets each of a reminder's settings to its latest value.
    fn refresh(&mut self, reminder_id: &str) {
        for setting in [Setting::Title, Setting::FireAt, Setting::Note] {
            let latest = self
                .versions
                .get(&(reminder_id.to_string(), setting))
                .and_then(|v| v.iter().max_by(|a, b| a.hlc.cmp(&b.hlc)))
                .map(|v| v.change.clone());
            let Some(r) = self.reminders.get_mut(reminder_id) else {
                return;
            };
            match latest {
                Some(Change::Title(t)) => r.title = t,
                Some(Change::FireAt(t)) => r.fire_at = t,
                Some(Change::Note(n)) => r.note = n,
                None => {}
            }
        }
    }

    /// The latest clock any setting change has carried: a new change is made
    /// after it.
    pub fn latest_hlc(&self) -> &Hlc {
        &self.latest_hlc
    }

    /// Every value a setting has had, the current one first and then the ones
    /// that lost, newest first. Each of them can be restored.
    pub fn history(&self, reminder_id: &str, setting: Setting) -> Vec<SettingVersion> {
        let mut v = self
            .versions
            .get(&(reminder_id.to_string(), setting))
            .cloned()
            .unwrap_or_default();
        v.sort_by(|a, b| b.hlc.cmp(&a.hlc));
        if let Some(first) = v.first_mut() {
            first.current = true;
        }
        v
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
            .filter(|o| o.is_open())
            .filter_map(|o| {
                let r = self.reminders.get(&o.reminder_id)?;
                Some(DueItem {
                    occurrence_id: o.id.clone(),
                    title: r.title.clone(),
                    scheduled_at: o.scheduled_at,
                    fired_at: o.fired_at,
                    not_sent: self.unsent_occurrences.contains(&o.id)
                        || self.unsent_reminders.contains(&r.id),
                    snoozed_until: o.snoozed_until,
                    acknowledged: o.acknowledged,
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
            .any(|o| o.reminder_id == reminder_id && !o.is_open())
    }
}
