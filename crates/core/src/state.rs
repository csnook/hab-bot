use std::collections::{BTreeMap, BTreeSet};

use jiff::tz::TimeZone;
use serde::Serialize;

use crate::countdown::Countdown;
use crate::delay::Delay;
use crate::event::{Change, Event, Setting, StoredEvent};
use crate::hlc::Hlc;
use crate::priority::{AlertStyle, Priority};
use crate::schedule::Schedule;

/// How long before a known expiry the last-chance alert comes: 10 minutes.
pub const LAST_CHANCE_LEAD: i64 = 600;

/// When the last-chance alert comes for a snooze made at `set_at` (`None`
/// if unknown) to end at `until`, for an occurrence that expires at
/// `expires_at`: 10 minutes before the expiry, if that moment falls inside
/// the snooze. A snooze made after that moment has no last-chance alert, the
/// picker having shown the expiry; the snooze then holds as chosen.
pub fn last_chance_at(expires_at: i64, set_at: Option<i64>, until: i64) -> Option<i64> {
    let at = expires_at.saturating_sub(LAST_CHANCE_LEAD);
    (until > at && set_at.is_none_or(|s| s < at)).then_some(at)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub id: String,
    pub list_id: String,
    pub title: String,
    pub fire_at: i64,
    pub note: String,
    /// What fires it, if it repeats. A reminder with none is a one-off,
    /// firing once at `fire_at`.
    pub schedules: Vec<Schedule>,
    /// The time zone the schedules are in. `None` is floating: wherever the
    /// device is.
    pub zone: Option<String>,
    /// When the reminder was made, in Unix seconds.
    pub created_at: i64,
    /// Schedules fire only for instants from here: when the reminder was made,
    /// or its schedules or zone were last changed. Setting a 7:00 reminder at
    /// 8:00 doesn't fire this morning's.
    pub active_from: i64,
    pub priority: Priority,
    /// Overrides the priority's due interval: when, counted from the
    /// scheduled time, an occurrence goes overdue.
    pub overdue_override: Option<Delay>,
    /// When, counted from the scheduled time, an open occurrence is missed:
    /// whichever comes first. Firing again always expires it too (ADR 0001).
    pub expiries: Vec<Delay>,
    /// What fires it, if it is a countdown: a set time after its last
    /// occurrence closed. A countdown reminder has no schedules.
    pub countdown: Option<Countdown>,
    /// When it was last done, as given when the countdown reminder was made,
    /// which starts its first countdown. `None` is "never": it fires at once.
    pub countdown_from: Option<i64>,
}

impl Reminder {
    /// The time zone the reminder's schedules are read in: its own, or else
    /// the device's.
    pub fn zone_in(&self, device: &TimeZone) -> TimeZone {
        self.zone
            .as_deref()
            .and_then(crate::schedule::zone)
            .unwrap_or_else(|| device.clone())
    }

    /// Seconds after an occurrence's scheduled time that it goes overdue by
    /// default: the priority's due interval.
    pub fn default_overdue_after(&self) -> i64 {
        self.priority.settings().due_interval
    }

    /// When an occurrence scheduled at `scheduled_at` goes overdue: the
    /// reminder's override if it has one that can be met, otherwise the
    /// priority's.
    pub fn overdue_at(&self, scheduled_at: i64, device: &TimeZone) -> i64 {
        let zone = self.zone_in(device);
        self.overdue_override
            .as_ref()
            .and_then(|d| d.resolve(scheduled_at, &zone))
            .unwrap_or_else(|| scheduled_at.saturating_add(self.default_overdue_after()))
    }

    /// When an occurrence scheduled at `scheduled_at` is missed for want of
    /// action, if the reminder has such an expiry: the first of its expiries.
    pub fn expires_at(&self, scheduled_at: i64, device: &TimeZone) -> Option<i64> {
        let zone = self.zone_in(device);
        self.expiries
            .iter()
            .filter_map(|d| d.resolve(scheduled_at, &zone))
            .min()
    }

    /// Fires again and again, on schedules or a countdown.
    pub fn repeats(&self) -> bool {
        self.has_schedules() || self.counts_down()
    }

    pub fn has_schedules(&self) -> bool {
        !self.schedules.is_empty()
    }

    pub fn counts_down(&self) -> bool {
        self.countdown.is_some()
    }
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
    /// When that snooze was made: the time recorded on its event.
    pub snoozed_at: Option<i64>,
    /// The current alert was silenced. Closing the occurrence ends it.
    pub acknowledged: bool,
    /// When it was last acknowledged: the time recorded on the
    /// acknowledging event, which is why that event needed no new format.
    /// Of several (on one device or several), the latest.
    pub acknowledged_at: Option<i64>,
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
    pub reminder_id: String,
    /// The list its reminder is in.
    pub list_id: String,
    pub title: String,
    /// The reminder's note, shown on its occurrences.
    pub note: String,
    pub scheduled_at: i64,
    pub fired_at: i64,
    /// A change to it hasn't been received by the server yet.
    pub not_sent: bool,
    pub snoozed_until: Option<i64>,
    /// When that snooze was made.
    pub snoozed_at: Option<i64>,
    pub acknowledged: bool,
    /// When it was last acknowledged, for the quiet period that follows.
    pub acknowledged_at: Option<i64>,
    pub priority: Priority,
    /// When it goes (or went) overdue, counted from `scheduled_at`.
    pub overdue_at: i64,
    /// When it is missed for want of action, if the reminder has such an
    /// expiry, counted from `scheduled_at`.
    pub expires_at: Option<i64>,
}

/// A reminder that has not fired yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpcomingItem {
    pub reminder_id: String,
    /// The list the reminder is in.
    pub list_id: String,
    pub priority: Priority,
    pub title: String,
    pub note: String,
    pub fire_at: i64,
    /// A change to it hasn't been received by the server yet.
    pub not_sent: bool,
}

/// How a snooze actually ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnoozeEnd {
    /// Its time came and alerts resumed.
    Elapsed,
    /// Another snooze took its place.
    Replaced,
    /// The occurrence was closed first.
    Closed,
}

/// One snooze, in the history: what it was set to end on and, once it has,
/// how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnoozeRecord {
    pub event_id: String,
    pub occurrence_id: String,
    /// The user who snoozed.
    pub user: String,
    /// The device that did, as it appears on its events.
    pub device_id: String,
    pub set_at: i64,
    /// What it was set to end on.
    pub until: i64,
    /// Made before the occurrence opened.
    pub ahead: bool,
    /// How and when it was cut short: replaced or closed over. A snooze that
    /// simply runs out has no entry here; see [`State::snoozes_of`].
    pub ended: Option<(i64, SnoozeEnd)>,
}

/// A snooze as the history shows it at a given moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnoozeView {
    pub occurrence_id: String,
    pub set_at: i64,
    pub until: i64,
    pub ahead: bool,
    /// `None` while it still holds.
    pub ended_at: Option<i64>,
    pub ended: Option<SnoozeEnd>,
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

/// An alert in the history: the first for an occurrence, or a change of style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertRecord {
    pub event_id: String,
    pub occurrence_id: String,
    /// The user who was alerted.
    pub user: String,
    /// The device that alerted, as it appears on its events. Other users see
    /// who, never which device.
    pub device_id: String,
    pub style: AlertStyle,
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

/// Who deleted a reminder, keeping its history, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    pub event_id: String,
    /// The user who deleted it.
    pub by: String,
    pub device_id: String,
    pub at: i64,
}

/// A reminder moved into this list, as the stream says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveIn {
    pub event_id: String,
    pub reminder_id: String,
    pub from_list_id: String,
    pub hlc: Hlc,
}

/// The current state, built by applying a stream's events in order.
#[derive(Debug, Default, Clone)]
pub struct State {
    /// What the list is called, if it has been named.
    pub list_name: Option<String>,
    /// The list's colour, `#rrggbb`, if it has been given one.
    pub list_colour: Option<String>,
    /// A `ListDeleted` event is in the stream. Whether the list is gone also
    /// depends on whether it still holds reminders ([`State::list_gone`]).
    pub list_deleted: bool,
    /// Reminders deleted with their history kept, with their settings as
    /// they were. Their occurrences stay in `occurrences` as history, but
    /// with no live reminder nothing fires, alerts or lists them.
    pub deleted: BTreeMap<String, Reminder>,
    /// Who deleted each reminder this stream says was deleted.
    pub tombstones: BTreeMap<String, Tombstone>,
    /// Reminders deleted with their history: nothing of them is kept.
    pub purged: BTreeSet<String>,
    /// The moves into this list, in stream order.
    pub moves_in: Vec<MoveIn>,
    /// Actions on an occurrence this list's stream doesn't have, such as one
    /// opened before its reminder was moved here. Applied again once the
    /// reminder's history has been taken in ([`State::replay_pending`]).
    pending: Vec<StoredEvent>,
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
    /// Alerts, in stream order: the first and each change of style, per user
    /// and device. Repeats aren't recorded.
    pub alerts: Vec<AlertRecord>,
    /// Every snooze, in stream order, each with what it was set to end on and
    /// how it ended. There is no limit on repeated snoozes.
    pub snoozes: Vec<SnoozeRecord>,
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
                | Event::RecurringReminderCreated { reminder_id, .. }
                | Event::CountdownReminderCreated { reminder_id, .. }
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
                Event::ExpectedOccurrenceSnoozed {
                    reminder_id,
                    scheduled_at,
                    ..
                } => {
                    self.unsent_occurrences
                        .insert(format!("{reminder_id}@{scheduled_at}"));
                }
                // An alert is history only: it isn't a change the user waits on.
                Event::OccurrenceAlerted { .. }
                | Event::ListNamed { .. }
                | Event::ListColoured { .. }
                | Event::ListDeleted
                | Event::ReminderMovedIn { .. }
                | Event::ReminderDeleted { .. }
                | Event::ReminderPurged { .. }
                | Event::DeviceNamed { .. }
                | Event::DeviceSignedIn { .. }
                | Event::DeviceRemoved { .. } => {}
            }
        }
        let target = match &stored.event {
            Event::OccurrenceCompleted { occurrence_id, .. }
            | Event::OccurrenceSkipped { occurrence_id, .. }
            | Event::OccurrenceMissed { occurrence_id, .. }
            | Event::OccurrenceSnoozed { occurrence_id, .. }
            | Event::OccurrenceAcknowledged { occurrence_id } => Some(occurrence_id),
            _ => None,
        };
        if let Some(id) = target {
            if !self.occurrences.contains_key(self.resolve(id)) {
                // Not opened in this stream: it may be in the list this
                // reminder came from. Kept until it has been taken in.
                self.pending.push(stored.clone());
                return;
            }
        }
        match &stored.event {
            Event::ReminderCreated {
                reminder_id,
                title,
                fire_at,
            } => {
                if self.can_create(reminder_id) {
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
                            schedules: Vec::new(),
                            zone: None,
                            created_at: stored.recorded_at,
                            active_from: stored.recorded_at,
                            priority: Priority::default(),
                            overdue_override: None,
                            expiries: Vec::new(),
                            countdown: None,
                            countdown_from: None,
                        },
                    );
                    self.refresh(reminder_id);
                }
            }
            Event::RecurringReminderCreated {
                reminder_id,
                title,
                schedules,
                zone,
            } => {
                if self.can_create(reminder_id) {
                    for change in [
                        Change::Title(title.clone()),
                        Change::Schedules(schedules.clone()),
                        Change::Zone(zone.clone()),
                    ] {
                        self.add_version(reminder_id, Hlc::default(), change, stored);
                    }
                    self.reminders.insert(
                        reminder_id.clone(),
                        Reminder {
                            id: reminder_id.clone(),
                            list_id: stored.list_id.clone(),
                            title: title.clone(),
                            fire_at: 0,
                            note: String::new(),
                            schedules: schedules.clone(),
                            zone: zone.clone(),
                            created_at: stored.recorded_at,
                            active_from: stored.recorded_at,
                            priority: Priority::default(),
                            overdue_override: None,
                            expiries: Vec::new(),
                            countdown: None,
                            countdown_from: None,
                        },
                    );
                    self.refresh(reminder_id);
                }
            }
            Event::CountdownReminderCreated {
                reminder_id,
                title,
                countdown,
                zone,
                last_done,
            } => {
                if self.can_create(reminder_id) {
                    for change in [
                        Change::Title(title.clone()),
                        Change::Countdown(countdown.clone()),
                        Change::Zone(zone.clone()),
                    ] {
                        self.add_version(reminder_id, Hlc::default(), change, stored);
                    }
                    self.reminders.insert(
                        reminder_id.clone(),
                        Reminder {
                            id: reminder_id.clone(),
                            list_id: stored.list_id.clone(),
                            title: title.clone(),
                            fire_at: 0,
                            note: String::new(),
                            schedules: Vec::new(),
                            zone: zone.clone(),
                            created_at: stored.recorded_at,
                            active_from: stored.recorded_at,
                            priority: Priority::default(),
                            overdue_override: None,
                            expiries: Vec::new(),
                            countdown: Some(countdown.clone()),
                            countdown_from: *last_done,
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
                if self.purged.contains(reminder_id) {
                    return;
                }
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
                    || self.purged.contains(reminder_id)
                {
                    return;
                }
                let one_off = self
                    .reminders
                    .get(reminder_id)
                    .is_some_and(|r| !r.repeats());
                if one_off {
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
                        snoozed_at: None,
                        acknowledged: false,
                        acknowledged_at: None,
                    },
                );
                // Snoozed ahead of time: it opens quietly.
                let ahead = self
                    .snoozes
                    .iter()
                    .rev()
                    .find(|s| &s.occurrence_id == occurrence_id && s.ended.is_none())
                    .map(|s| (s.until, s.set_at));
                if let (Some((until, at)), Some(o)) =
                    (ahead, self.occurrences.get_mut(occurrence_id))
                {
                    o.snoozed_until = Some(until);
                    o.snoozed_at = Some(at);
                }
                if !one_off {
                    self.expire_superseded(reminder_id);
                }
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
            } => self.snooze(occurrence_id, *until, false, stored),
            Event::ExpectedOccurrenceSnoozed {
                reminder_id,
                scheduled_at,
                until,
            } if !self.purged.contains(reminder_id) => self.snooze(
                &format!("{reminder_id}@{scheduled_at}"),
                *until,
                true,
                stored,
            ),
            Event::OccurrenceAcknowledged { occurrence_id } => {
                if let Some(o) = self.open_occurrence_mut(occurrence_id) {
                    o.acknowledged = true;
                    o.acknowledged_at = o.acknowledged_at.max(Some(stored.recorded_at));
                }
            }
            Event::OccurrenceAlerted {
                occurrence_id,
                style,
            } => {
                let occurrence_id = self.resolve(occurrence_id).to_string();
                if !self.alerts.iter().any(|a| a.event_id == stored.event_id) {
                    self.alerts.push(AlertRecord {
                        event_id: stored.event_id.clone(),
                        occurrence_id,
                        user: stored.author.clone(),
                        device_id: stored.device_id.clone(),
                        style: *style,
                        at: stored.recorded_at,
                    });
                }
            }
            // Purged: nothing is kept of what happens to it.
            Event::ExpectedOccurrenceSnoozed { .. } => {}
            Event::ListNamed { name } => {
                self.list_name = Some(name.clone());
            }
            Event::ListColoured { colour } => {
                self.list_colour = Some(colour.clone());
            }
            Event::ListDeleted => {
                self.list_deleted = true;
            }
            Event::ReminderMovedIn {
                reminder_id,
                from_list_id,
                hlc,
            } => {
                if self.purged.contains(reminder_id)
                    || self.moves_in.iter().any(|m| m.event_id == stored.event_id)
                {
                    return;
                }
                let hlc = hlc.clamped(stored.recorded_at);
                if hlc > self.latest_hlc {
                    self.latest_hlc = hlc.clone();
                }
                self.moves_in.push(MoveIn {
                    event_id: stored.event_id.clone(),
                    reminder_id: reminder_id.clone(),
                    from_list_id: from_list_id.clone(),
                    hlc,
                });
            }
            Event::ReminderDeleted { reminder_id } => {
                if self.purged.contains(reminder_id) {
                    return;
                }
                self.add_tombstone(
                    reminder_id,
                    Tombstone {
                        event_id: stored.event_id.clone(),
                        by: stored.author.clone(),
                        device_id: stored.device_id.clone(),
                        at: stored.recorded_at,
                    },
                );
            }
            Event::ReminderPurged { reminder_id } => {
                self.purged.insert(reminder_id.clone());
                self.drop_reminder(reminder_id);
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

    /// Records a snooze and, if the occurrence is open, applies it. Closing
    /// beats snoozing, whichever arrives first. An occurrence that doesn't
    /// exist yet takes a snooze made ahead of time when it opens; any other
    /// snooze of an unknown occurrence is ignored.
    fn snooze(&mut self, occurrence_id: &str, until: i64, ahead: bool, stored: &StoredEvent) {
        let id = self.resolve(occurrence_id).to_string();
        if self.snoozes.iter().any(|s| s.event_id == stored.event_id) {
            return;
        }
        let set_at = stored.recorded_at;
        let mut ended = None;
        match self.occurrences.get_mut(&id) {
            Some(o) => match &o.closing {
                Some(c) => ended = Some((c.at, SnoozeEnd::Closed)),
                None => {
                    o.snoozed_until = Some(until);
                    o.snoozed_at = Some(set_at);
                }
            },
            None if ahead => {}
            None => return,
        }
        if ended.is_none() {
            // Another snooze takes the place of the one holding.
            for s in &mut self.snoozes {
                if s.occurrence_id == id && s.ended.is_none() {
                    s.ended = Some((set_at, SnoozeEnd::Replaced));
                }
            }
        }
        self.snoozes.push(SnoozeRecord {
            event_id: stored.event_id.clone(),
            occurrence_id: id,
            user: stored.author.clone(),
            device_id: stored.device_id.clone(),
            set_at,
            until,
            ahead,
            ended,
        });
    }

    /// Ends the snoozes holding on an occurrence that closed at `at`.
    fn end_snoozes(&mut self, occurrence_id: &str, at: i64) {
        for s in &mut self.snoozes {
            if s.occurrence_id == occurrence_id && s.ended.is_none() {
                s.ended = Some((at, SnoozeEnd::Closed));
            }
        }
    }

    /// The occurrence's snoozes, oldest first, as they stand at `now`: one
    /// whose time has come without being replaced or closed over has ended
    /// by elapsing, at that time.
    pub fn snoozes_of(&self, occurrence_id: &str, now: i64) -> Vec<SnoozeView> {
        let id = self.resolve(occurrence_id);
        self.snoozes
            .iter()
            .filter(|s| s.occurrence_id == id)
            .map(|s| {
                let (ended_at, ended) = match s.ended {
                    Some((at, how)) => (Some(at), Some(how)),
                    None if s.until <= now => (Some(s.until), Some(SnoozeEnd::Elapsed)),
                    None => (None, None),
                };
                SnoozeView {
                    occurrence_id: s.occurrence_id.clone(),
                    set_at: s.set_at,
                    until: s.until,
                    ahead: s.ahead,
                    ended_at,
                    ended,
                }
            })
            .collect()
    }

    /// Until when an expected occurrence has been snoozed ahead of time.
    pub fn expected_snooze(&self, reminder_id: &str, scheduled_at: i64) -> Option<i64> {
        let id = format!("{reminder_id}@{scheduled_at}");
        self.snoozes
            .iter()
            .rev()
            .find(|s| s.occurrence_id == id && s.ended.is_none())
            .map(|s| s.until)
    }

    /// The style `device_id` last recorded an alert in for the occurrence.
    pub fn last_alert_style(&self, occurrence_id: &str, device_id: &str) -> Option<AlertStyle> {
        let id = self.resolve(occurrence_id);
        self.alerts
            .iter()
            .rev()
            .find(|a| a.occurrence_id == id && a.device_id == device_id)
            .map(|a| a.style)
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
        let new = Closing {
            kind,
            by: stored.author.clone(),
            device_id: stored.device_id.clone(),
            at,
            note,
            event_id: stored.event_id.clone(),
        };
        self.merge_closing(&id, new);
    }

    /// Gives an occurrence a closing, or settles between it and the one it
    /// has by the rules above.
    fn merge_closing(&mut self, id: &str, new: Closing) {
        let Some(o) = self.occurrences.get_mut(id) else {
            return;
        };
        let (winner, loser) = match o.closing.take() {
            None => {
                let at = new.at;
                o.snoozed_until = None;
                o.snoozed_at = None;
                o.acknowledged = false;
                o.acknowledged_at = None;
                o.closing = Some(new);
                self.end_snoozes(id, at);
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
                occurrence_id: id.to_string(),
                device_id: loser.device_id.clone(),
                by: loser.by.clone(),
                lost: loser.kind,
                lost_at: loser.at,
            });
        }
        if let Some(o) = self.occurrences.get_mut(id) {
            o.closing = Some(winner);
        }
    }

    /// Keeps a repeating reminder to one open occurrence (ADR 0001), whatever
    /// order its openings arrive in: only the latest instance can be open. An
    /// older one is closed as missed, as of when the next instance was due.
    /// Nobody did this, so it names no user and no device.
    fn expire_superseded(&mut self, reminder_id: &str) {
        let Some(newest) = self
            .occurrences
            .values()
            .filter(|o| o.reminder_id == reminder_id)
            .map(|o| (o.scheduled_at, o.id.clone()))
            .max()
        else {
            return;
        };
        let mut ended = Vec::new();
        for o in self.occurrences.values_mut() {
            if o.reminder_id == reminder_id && o.id != newest.1 && o.closing.is_none() {
                ended.push((o.id.clone(), newest.0.max(o.scheduled_at)));
                o.snoozed_until = None;
                o.snoozed_at = None;
                o.acknowledged = false;
                o.acknowledged_at = None;
                o.closing = Some(Closing {
                    kind: ClosingKind::Missed,
                    by: String::new(),
                    device_id: String::new(),
                    at: newest.0.max(o.scheduled_at),
                    note: None,
                    event_id: format!("expired:{}", o.id),
                });
            }
        }
        for (id, at) in ended {
            self.end_snoozes(&id, at);
        }
    }

    /// The reminder's latest occurrence: the one a countdown restarts from.
    pub fn latest_occurrence(&self, reminder_id: &str) -> Option<&Occurrence> {
        self.occurrences
            .values()
            .filter(|o| o.reminder_id == reminder_id)
            .max_by(|a, b| (a.scheduled_at, &a.id).cmp(&(b.scheduled_at, &b.id)))
    }

    /// The scheduled time of the reminder's latest occurrence.
    pub fn last_scheduled(&self, reminder_id: &str) -> Option<i64> {
        self.occurrences
            .values()
            .filter(|o| o.reminder_id == reminder_id)
            .map(|o| o.scheduled_at)
            .max()
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
        let mut active_from = None;
        for setting in [
            Setting::Title,
            Setting::FireAt,
            Setting::Note,
            Setting::Schedules,
            Setting::Zone,
            Setting::Priority,
            Setting::Overdue,
            Setting::Expiry,
            Setting::Countdown,
        ] {
            let latest = self
                .versions
                .get(&(reminder_id.to_string(), setting))
                .and_then(|v| v.iter().max_by(|a, b| a.hlc.cmp(&b.hlc)));
            if matches!(setting, Setting::Schedules | Setting::Zone) {
                if let Some(v) = latest.filter(|v| v.hlc != Hlc::default()) {
                    active_from = active_from.max(Some(v.recorded_at));
                }
            }
            let latest = latest.map(|v| v.change.clone());
            let Some(r) = self.reminders.get_mut(reminder_id) else {
                return;
            };
            match latest {
                Some(Change::Title(t)) => r.title = t,
                Some(Change::FireAt(t)) => r.fire_at = t,
                Some(Change::Note(n)) => r.note = n,
                Some(Change::Schedules(s)) => r.schedules = s,
                Some(Change::Zone(z)) => r.zone = z,
                Some(Change::Priority(p)) => r.priority = p,
                Some(Change::Overdue(o)) => r.overdue_override = o,
                Some(Change::Expiry(e)) => r.expiries = e,
                Some(Change::Countdown(c)) => r.countdown = Some(c),
                None => {}
            }
        }
        if let (Some(r), Some(at)) = (self.reminders.get_mut(reminder_id), active_from) {
            r.active_from = r.active_from.max(at);
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
            .filter(|r| !r.repeats() && r.fire_at <= now && !self.has_fired(&r.id))
            .collect();
        v.sort_by_key(|r| (r.fire_at, r.id.clone()));
        v
    }

    /// The earliest time an unfired reminder is due.
    pub fn next_fire_at(&self) -> Option<i64> {
        self.reminders
            .values()
            .filter(|r| !r.repeats() && !self.has_fired(&r.id))
            .map(|r| r.fire_at)
            .min()
    }

    /// Every open occurrence, oldest firing first, due or overdue alike. The
    /// Inbox splits them ([`crate::Core::inbox`]).
    pub fn due(&self, device: &TimeZone) -> Vec<DueItem> {
        let mut v: Vec<DueItem> = self
            .occurrences
            .values()
            .filter(|o| o.is_open())
            .filter_map(|o| {
                let r = self.reminders.get(&o.reminder_id)?;
                Some(DueItem {
                    occurrence_id: o.id.clone(),
                    reminder_id: r.id.clone(),
                    list_id: r.list_id.clone(),
                    title: r.title.clone(),
                    note: r.note.clone(),
                    scheduled_at: o.scheduled_at,
                    fired_at: o.fired_at,
                    not_sent: self.unsent_occurrences.contains(&o.id)
                        || self.unsent_reminders.contains(&r.id),
                    snoozed_until: o.snoozed_until,
                    snoozed_at: o.snoozed_at,
                    acknowledged: o.acknowledged,
                    acknowledged_at: o.acknowledged_at,
                    priority: r.priority,
                    overdue_at: r.overdue_at(o.scheduled_at, device),
                    expires_at: r.expires_at(o.scheduled_at, device),
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
            .filter(|r| !r.repeats() && !self.has_fired(&r.id))
            .map(|r| UpcomingItem {
                reminder_id: r.id.clone(),
                list_id: r.list_id.clone(),
                priority: r.priority,
                title: r.title.clone(),
                note: r.note.clone(),
                fire_at: r.fire_at,
                not_sent: self.unsent_reminders.contains(&r.id),
            })
            .collect();
        v.sort_by_key(|u| (u.fire_at, u.reminder_id.clone()));
        v
    }

    /// A reminder with this id can be made here: it isn't already here, or
    /// deleted, and nothing of it was purged.
    fn can_create(&self, reminder_id: &str) -> bool {
        !self.reminders.contains_key(reminder_id)
            && !self.deleted.contains_key(reminder_id)
            && !self.purged.contains(reminder_id)
            && !self.tombstones.contains_key(reminder_id)
    }

    /// Records that a reminder was deleted, keeping the earliest deletion, and
    /// takes it out of the live reminders.
    fn add_tombstone(&mut self, reminder_id: &str, tombstone: Tombstone) {
        let earlier = self
            .tombstones
            .get(reminder_id)
            .is_none_or(|t| (tombstone.at, &tombstone.event_id) < (t.at, &t.event_id));
        if earlier {
            self.tombstones.insert(reminder_id.to_string(), tombstone);
        }
        self.bury(reminder_id);
    }

    /// Moves a reminder with a tombstone out of the live reminders. Whatever
    /// else comes to it, it stays deleted.
    fn bury(&mut self, reminder_id: &str) {
        if !self.tombstones.contains_key(reminder_id) {
            return;
        }
        if let Some(r) = self.reminders.remove(reminder_id) {
            self.deleted.insert(reminder_id.to_string(), r);
        }
    }

    /// Forgets everything about a reminder: its settings and their history,
    /// occurrences, snoozes, alerts and notices.
    pub(crate) fn drop_reminder(&mut self, reminder_id: &str) {
        self.reminders.remove(reminder_id);
        self.deleted.remove(reminder_id);
        self.tombstones.remove(reminder_id);
        self.moves_in.retain(|m| m.reminder_id != reminder_id);
        self.versions.retain(|(id, _), _| id != reminder_id);
        self.unsent_reminders.remove(reminder_id);
        let prefix = format!("{reminder_id}@");
        let gone: BTreeSet<String> = self
            .occurrences
            .values()
            .filter(|o| o.reminder_id == reminder_id)
            .map(|o| o.id.clone())
            .collect();
        let mine = |id: &String| gone.contains(id) || id.starts_with(&prefix);
        self.occurrences.retain(|id, _| !mine(id));
        self.aliases
            .retain(|alias, target| !mine(alias) && !mine(target));
        self.snoozes.retain(|s| !mine(&s.occurrence_id));
        self.alerts.retain(|a| !mine(&a.occurrence_id));
        self.reconciliations.retain(|r| !mine(&r.occurrence_id));
        self.unsent_occurrences.retain(|id| !mine(id));
    }

    /// Applies again the actions that found no occurrence when they came,
    /// now that reminders moved here have brought theirs.
    pub(crate) fn replay_pending(&mut self) {
        for stored in std::mem::take(&mut self.pending) {
            self.apply(&stored);
        }
    }

    /// Whether the reminder is here, live or deleted.
    pub fn knows(&self, reminder_id: &str) -> bool {
        self.reminders.contains_key(reminder_id) || self.deleted.contains_key(reminder_id)
    }

    /// When the reminder came into this list, if it was moved here: the
    /// latest such move's clock.
    pub fn moved_in_at(&self, reminder_id: &str) -> Option<&Hlc> {
        self.moves_in
            .iter()
            .filter(|m| m.reminder_id == reminder_id)
            .map(|m| &m.hlc)
            .max()
    }

    /// The list was deleted, and nothing in it is live: a reminder moved or
    /// made into it by another device since keeps it (ADR 0009).
    pub fn list_gone(&self) -> bool {
        self.list_deleted && self.reminders.is_empty()
    }

    /// Takes in everything `from` (a list the reminder was in before) has
    /// of the reminder, which this list now holds: its settings with their
    /// history, its occurrences and what was done to them, and its deletion.
    /// What each list has is a union, so it doesn't matter in which order
    /// devices' events reached either list, or how often this runs.
    pub(crate) fn absorb(&mut self, from: &State, reminder_id: &str, to_list: &str) {
        let Some(source) = from
            .reminders
            .get(reminder_id)
            .or_else(|| from.deleted.get(reminder_id))
        else {
            return;
        };
        if !self.knows(reminder_id) {
            let mut r = source.clone();
            r.list_id = to_list.to_string();
            self.reminders.insert(reminder_id.to_string(), r);
        }
        // Settings, with every value they ever had.
        for ((id, setting), versions) in &from.versions {
            if id != reminder_id {
                continue;
            }
            let mine = self.versions.entry((id.clone(), *setting)).or_default();
            for v in versions {
                if !mine
                    .iter()
                    .any(|m| m.event_id == v.event_id && m.hlc == v.hlc)
                {
                    mine.push(v.clone());
                }
            }
        }
        if from.latest_hlc > self.latest_hlc {
            self.latest_hlc = from.latest_hlc.clone();
        }
        // Occurrences, each closed or open as the union says.
        let prefix = format!("{reminder_id}@");
        let belongs = |id: &String| id.starts_with(&prefix);
        let theirs: Vec<&Occurrence> = from
            .occurrences
            .values()
            .filter(|o| o.reminder_id == reminder_id)
            .collect();
        for o in theirs {
            match self.occurrences.get_mut(&o.id) {
                None => {
                    self.occurrences.insert(o.id.clone(), o.clone());
                }
                Some(mine) => {
                    if mine.closing.is_none() {
                        if o.snoozed_at > mine.snoozed_at {
                            mine.snoozed_until = o.snoozed_until;
                            mine.snoozed_at = o.snoozed_at;
                        }
                        mine.acknowledged |= o.acknowledged;
                        mine.acknowledged_at = mine.acknowledged_at.max(o.acknowledged_at);
                    }
                }
            }
            if let Some(c) = &o.closing {
                self.merge_closing(&o.id, c.clone());
            }
        }
        for (alias, target) in &from.aliases {
            if belongs(alias) || belongs(target) {
                self.aliases
                    .entry(alias.clone())
                    .or_insert_with(|| target.clone());
            }
        }
        for s in from.snoozes.iter().filter(|s| belongs(&s.occurrence_id)) {
            if !self.snoozes.iter().any(|m| m.event_id == s.event_id) {
                self.snoozes.push(s.clone());
            }
        }
        for a in from.alerts.iter().filter(|a| belongs(&a.occurrence_id)) {
            if !self.alerts.iter().any(|m| m.event_id == a.event_id) {
                self.alerts.push(a.clone());
            }
        }
        for r in from
            .reconciliations
            .iter()
            .filter(|r| belongs(&r.occurrence_id))
        {
            if !self.reconciliations.iter().any(|m| m.id == r.id) {
                self.reconciliations.push(r.clone());
            }
        }
        if from.unsent_reminders.contains(reminder_id) {
            self.unsent_reminders.insert(reminder_id.to_string());
        }
        self.unsent_occurrences.extend(
            from.unsent_occurrences
                .iter()
                .filter(|id| belongs(id))
                .cloned(),
        );
        // A deletion goes along with the reminder, whatever else happened.
        if let Some(t) = from.tombstones.get(reminder_id) {
            self.add_tombstone(reminder_id, t.clone());
        }
        self.bury(reminder_id);
        self.refresh(reminder_id);
        // Merging closings can leave a repeating reminder with two open.
        if self.reminders.get(reminder_id).is_some_and(|r| r.repeats()) {
            self.expire_superseded(reminder_id);
        }
    }

    /// Reminders deleted with their history kept, newest deletion first.
    pub fn deleted_reminders(&self) -> Vec<(&Reminder, &Tombstone)> {
        let mut v: Vec<(&Reminder, &Tombstone)> = self
            .deleted
            .values()
            .filter_map(|r| Some((r, self.tombstones.get(&r.id)?)))
            .collect();
        v.sort_by(|a, b| (b.1.at, &b.0.id).cmp(&(a.1.at, &a.0.id)));
        v
    }

    /// Whether any change hasn't been received by the server yet.
    pub fn has_unsent(&self) -> bool {
        !self.unsent_reminders.is_empty() || !self.unsent_occurrences.is_empty()
    }

    /// A one-off reminder is finished once its occurrence is closed.
    pub fn is_finished(&self, reminder_id: &str) -> bool {
        self.reminders
            .get(reminder_id)
            .is_some_and(|r| !r.repeats())
            && self
                .occurrences
                .values()
                .any(|o| o.reminder_id == reminder_id && !o.is_open())
    }
}
