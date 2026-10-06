use std::collections::{BTreeMap, BTreeSet};

use jiff::tz::TimeZone;
use serde::Serialize;

use crate::condition::Condition;
use crate::countdown::Countdown;
use crate::delay::Delay;
use crate::event::{Change, Correction, Event, Setting, StoredEvent, UndoOutcome};
use crate::hlc::Hlc;
use crate::pause::{Pause, PauseCause};
use crate::place::Place;
use crate::priority::{AlertStyle, Priority};
use crate::quiet::{self, Hold, QuietHours, SnoozeAll, Source};
use crate::schedule::Schedule;
use crate::sun::SunTrigger;

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
    /// Set aside for a period, if it is. A reminder in a paused list is also
    /// paused, by the list ([`State::pause_at`]).
    pub pause: Option<Pause>,
    /// Sun-event triggers, which fire it at a sun event at the user's home,
    /// as well as its schedules.
    pub suns: Vec<SunTrigger>,
    /// Time-based conditions, combined with AND: an instant outside them
    /// passes, without firing or waiting.
    pub conditions: Vec<Condition>,
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

    /// Fires again and again, on schedules, sun events or a countdown.
    pub fn repeats(&self) -> bool {
        self.has_schedules() || self.counts_down()
    }

    /// Fires on schedules or sun events (scheduled instances that can be
    /// predicted), rather than once or after a countdown.
    pub fn has_schedules(&self) -> bool {
        !self.schedules.is_empty() || !self.suns.is_empty()
    }

    /// Needs the home location to work out when it fires or whether it may:
    /// it has a sun event or a daylight or darkness condition.
    pub fn needs_home(&self) -> bool {
        !self.suns.is_empty() || self.conditions.iter().any(Condition::needs_home)
    }

    /// How easily its triggers and conditions can be faked. Every kind in
    /// this release (a time, a schedule, a countdown, a sun event, a time of
    /// day or daylight) is worked out from the clock and the home location,
    /// so none can be; sensed kinds (Wi-Fi, Bluetooth, a webhook) will say.
    pub fn faking(&self) -> crate::core::Faking {
        crate::core::Faking::None
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
    /// When the event was recorded, which is when it was tapped: `at` is the
    /// time the user said.
    pub recorded_at: i64,
    /// The closings the author had seen and took the place of: a correction,
    /// or the miss an undo left. Empty for a closing of an open occurrence.
    pub replaces: Vec<String>,
    /// A skip made because the occurrence fell in a pause.
    pub paused: Option<PauseCause>,
}

impl Closing {
    /// A closing made by the event `stored`, replacing nothing.
    fn new(kind: ClosingKind, at: i64, note: Option<String>, stored: &StoredEvent) -> Closing {
        Closing {
            kind,
            by: stored.author.clone(),
            device_id: stored.device_id.clone(),
            at,
            note,
            event_id: stored.event_id.clone(),
            recorded_at: stored.recorded_at,
            replaces: Vec::new(),
            paused: None,
        }
    }
}

/// The ids of the closings and undos that a correction or an undo has taken
/// the place of, as long as that correction or undo itself still stands. (A
/// correction or undo made here names everything it replaces, so only two
/// devices acting on one closing can leave one standing over another.)
fn superseded<'a>(records: &'a [Closing], undos: &'a [Undo]) -> BTreeSet<&'a str> {
    let mut by: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let adjusters = records
        .iter()
        .map(|c| (c.event_id.as_str(), &c.replaces))
        .chain(undos.iter().map(|u| (u.event_id.as_str(), &u.replaces)));
    for (id, replaces) in adjusters {
        for target in replaces {
            by.entry(target.as_str()).or_default().push(id);
        }
    }
    fn dead<'a>(
        id: &'a str,
        by: &BTreeMap<&'a str, Vec<&'a str>>,
        memo: &mut BTreeMap<&'a str, bool>,
        path: &mut Vec<&'a str>,
    ) -> bool {
        if let Some(d) = memo.get(id) {
            return *d;
        }
        // A cycle can't come from honest devices; it replaces nothing.
        if path.contains(&id) {
            return false;
        }
        path.push(id);
        let d = by
            .get(id)
            .is_some_and(|a| a.iter().any(|a| !dead(a, by, memo, path)));
        path.pop();
        memo.insert(id, d);
        d
    }
    let mut memo = BTreeMap::new();
    by.keys()
        .copied()
        .filter(|id| dead(id, &by, &mut memo, &mut Vec::new()))
        .collect()
}

/// An undo that left the occurrence without a closing: reopened, or back to
/// being expected. (An undo that left it missed is a [`Closing`] that
/// replaces the one undone.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undo {
    pub event_id: String,
    pub by: String,
    pub device_id: String,
    pub recorded_at: i64,
    pub replaces: Vec<String>,
    pub outcome: UndoOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub id: String,
    pub reminder_id: String,
    pub scheduled_at: i64,
    pub fired_at: i64,
    /// How it stands closed: of the closings that haven't been replaced by a
    /// correction or an undo, the one that counts ([`State::settle`]).
    pub closing: Option<Closing>,
    /// Every closing, including the ones corrections and undos replaced,
    /// which the history keeps. Oldest first.
    pub records: Vec<Closing>,
    /// The undos that left no closing.
    pub undos: Vec<Undo>,
    /// Closed ahead of its time and then undone: it is expected again and
    /// fires at its time as usual.
    pub unfired: bool,
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

    /// Fired and not closed.
    pub fn is_open(&self) -> bool {
        self.closing.is_none() && !self.unfired
    }

    /// Opened ahead of its time, to be closed early.
    pub fn is_early(&self) -> bool {
        self.fired_at < self.scheduled_at
    }
}

/// What the history shows of one thing done to an occurrence's closing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub event_id: String,
    pub what: HistoryWhat,
    /// The time the user said, for a closing.
    pub at: Option<i64>,
    pub note: Option<String>,
    /// Who did it; empty if the app did.
    pub by: String,
    pub device_id: String,
    /// When it was tapped.
    pub recorded_at: i64,
    /// A later correction or undo took its place.
    pub superseded: bool,
    /// It is a correction of, or an undo of, an earlier entry.
    pub replaces: Vec<String>,
    /// A skip the pause made, which the history attributes to it.
    pub paused: Option<PauseCause>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryWhat {
    Completed,
    Skipped,
    Missed,
    /// Undone: it reopened.
    Reopened,
    /// Undone: it is expected again.
    Expected,
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
    /// The snooze-all it was part of was ended early.
    Ended,
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
    /// `None` for a snooze made on the occurrence; otherwise what held it: a
    /// snooze-all or quiet hours, worked out from the settings
    /// ([`State::holds_over`]).
    pub via: Option<Source>,
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

/// One pause or resume of a whole list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPauseVersion {
    pub event_id: String,
    pub hlc: Hlc,
    /// `None` resumed the list.
    pub pause: Option<Pause>,
    pub by: String,
    pub device_id: String,
    pub recorded_at: i64,
}

/// One setting or clearing of the user's home location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeVersion {
    pub event_id: String,
    pub hlc: Hlc,
    /// `None` cleared the home location.
    pub place: Option<Place>,
    pub recorded_at: i64,
}

/// A snooze-all, as the personal list's stream says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnoozeAllRecord {
    pub event_id: String,
    pub snooze: SnoozeAll,
    pub by: String,
    pub device_id: String,
    pub recorded_at: i64,
    /// When it was ended early, if it was.
    pub ended: Option<i64>,
}

impl SnoozeAllRecord {
    /// When it stops holding: its end, or the time it was ended if earlier.
    pub fn effective_until(&self) -> i64 {
        self.ended
            .map_or(self.snooze.until, |e| e.min(self.snooze.until))
    }
}

/// One setting of the user's quiet hours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietHoursVersion {
    pub event_id: String,
    pub hlc: Hlc,
    pub rules: Vec<QuietHours>,
    pub recorded_at: i64,
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
    /// Every pause or resume of this whole list, in stream order.
    pub list_pauses: Vec<ListPauseVersion>,
    /// The list's pause now: the latest of them by clock. Reminders in the
    /// list are paused by it for as long as it covers them.
    pub list_pause: Option<Pause>,
    /// Every setting or clearing of the user's home location this stream
    /// holds. Only the personal list's count (ADR 0012).
    pub homes: Vec<HomeVersion>,
    /// The home location now: the latest of them by clock.
    pub home: Option<Place>,
    /// When the latest of them was made (as the device recorded it): sun
    /// events fire only for instants after it, so setting a home doesn't
    /// fire what has passed that day.
    pub home_since: Option<i64>,
    /// Every snooze-all this stream holds, in stream order. Only the personal
    /// list's count (ADR 0013).
    pub snooze_alls: Vec<SnoozeAllRecord>,
    /// When each snooze-all was ended, by id, including ones whose start
    /// hasn't been seen.
    snooze_all_ends: BTreeMap<String, i64>,
    /// Every setting of the user's quiet hours this stream holds.
    pub quiet_versions: Vec<QuietHoursVersion>,
    /// The quiet hours now: the latest setting by clock.
    pub quiet_hours: Vec<QuietHours>,
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
    /// Whether each device (by id) says it is portable.
    pub device_portable: BTreeMap<String, bool>,
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
                | Event::OccurrenceSkippedForPause { occurrence_id, .. }
                | Event::OccurrenceCorrected { occurrence_id, .. }
                | Event::OccurrenceUndone { occurrence_id, .. }
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
                | Event::ListPaused { .. }
                | Event::HomeSet { .. }
                | Event::SnoozeAllStarted { .. }
                | Event::SnoozeAllEnded { .. }
                | Event::QuietHoursSet { .. }
                | Event::ReminderMovedIn { .. }
                | Event::ReminderDeleted { .. }
                | Event::ReminderPurged { .. }
                | Event::DeviceNamed { .. }
                | Event::DevicePortable { .. }
                | Event::DeviceSignedIn { .. }
                | Event::DeviceRemoved { .. } => {}
            }
        }
        let target = match &stored.event {
            Event::OccurrenceCompleted { occurrence_id, .. }
            | Event::OccurrenceSkipped { occurrence_id, .. }
            | Event::OccurrenceMissed { occurrence_id, .. }
            | Event::OccurrenceCorrected { occurrence_id, .. }
            | Event::OccurrenceUndone { occurrence_id, .. }
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
                            pause: None,
                            suns: Vec::new(),
                            conditions: Vec::new(),
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
                suns,
                conditions,
            } => {
                if self.can_create(reminder_id) {
                    for change in [
                        Change::Title(title.clone()),
                        Change::Schedules(schedules.clone()),
                        Change::Zone(zone.clone()),
                        Change::SunEvents(suns.clone()),
                        Change::Conditions(conditions.clone()),
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
                            pause: None,
                            suns: suns.clone(),
                            conditions: conditions.clone(),
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
                            pause: None,
                            suns: Vec::new(),
                            conditions: Vec::new(),
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
                if self.purged.contains(reminder_id) {
                    return;
                }
                if let Some(o) = self.occurrences.get_mut(occurrence_id) {
                    // Opened ahead of its time on one device, and now really
                    // fired on another (or here, after an undo).
                    if o.is_early() && *fired_at >= o.scheduled_at {
                        o.fired_at = *fired_at;
                        self.settle(occurrence_id);
                    }
                    return;
                }
                self.insert_occurrence(occurrence_id, reminder_id, *scheduled_at, *fired_at);
            }
            Event::OccurrenceSkippedForPause {
                occurrence_id,
                reminder_id,
                scheduled_at,
                skipped_at,
                until,
                list,
            } => {
                if self.purged.contains(reminder_id) {
                    return;
                }
                // An instance that came due while paused is opened, closed
                // at once; an occurrence open when the pause began is closed.
                if !self.occurrences.contains_key(self.resolve(occurrence_id)) {
                    self.insert_occurrence(
                        occurrence_id,
                        reminder_id,
                        *scheduled_at,
                        *scheduled_at,
                    );
                }
                self.add_record(
                    occurrence_id,
                    Closing {
                        paused: Some(PauseCause {
                            until: *until,
                            list: *list,
                        }),
                        ..Closing::new(ClosingKind::Skipped, *skipped_at, None, stored)
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
            Event::OccurrenceCorrected {
                occurrence_id,
                replaces,
                kind,
                at,
                note,
            } => {
                let kind = match kind {
                    Correction::Completed => ClosingKind::Completed,
                    Correction::Skipped => ClosingKind::Skipped,
                };
                let note = note.clone().filter(|_| kind == ClosingKind::Skipped);
                self.add_record(
                    occurrence_id,
                    Closing {
                        replaces: replaces.clone(),
                        ..Closing::new(kind, *at, note, stored)
                    },
                );
            }
            Event::OccurrenceUndone {
                occurrence_id,
                replaces,
                outcome,
            } => {
                let id = self.resolve(occurrence_id).to_string();
                match outcome {
                    UndoOutcome::Missed { at } => self.add_record(
                        &id,
                        Closing {
                            replaces: replaces.clone(),
                            ..Closing::new(ClosingKind::Missed, *at, None, stored)
                        },
                    ),
                    _ => {
                        if let Some(o) = self.occurrences.get_mut(&id) {
                            if !o.undos.iter().any(|u| u.event_id == stored.event_id) {
                                o.undos.push(Undo {
                                    event_id: stored.event_id.clone(),
                                    by: stored.author.clone(),
                                    device_id: stored.device_id.clone(),
                                    recorded_at: stored.recorded_at,
                                    replaces: replaces.clone(),
                                    outcome: outcome.clone(),
                                });
                            }
                        }
                        self.settle(&id);
                    }
                }
            }
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
            Event::ListPaused { hlc, pause } => {
                if self
                    .list_pauses
                    .iter()
                    .any(|p| p.event_id == stored.event_id)
                {
                    return;
                }
                let hlc = hlc.clamped(stored.recorded_at);
                if hlc > self.latest_hlc {
                    self.latest_hlc = hlc.clone();
                }
                self.list_pauses.push(ListPauseVersion {
                    event_id: stored.event_id.clone(),
                    hlc,
                    pause: *pause,
                    by: stored.author.clone(),
                    device_id: stored.device_id.clone(),
                    recorded_at: stored.recorded_at,
                });
                // Of several, the latest clock counts, whatever order they
                // arrive in.
                self.list_pause = self
                    .list_pauses
                    .iter()
                    .max_by(|a, b| a.hlc.cmp(&b.hlc))
                    .and_then(|p| p.pause);
            }
            Event::HomeSet { hlc, place } => {
                if self.homes.iter().any(|h| h.event_id == stored.event_id) {
                    return;
                }
                let hlc = hlc.clamped(stored.recorded_at);
                if hlc > self.latest_hlc {
                    self.latest_hlc = hlc.clone();
                }
                self.homes.push(HomeVersion {
                    event_id: stored.event_id.clone(),
                    hlc,
                    place: place.clone(),
                    recorded_at: stored.recorded_at,
                });
                // Of several, the latest clock counts, whatever order they
                // arrive in.
                let latest = self.homes.iter().max_by(|a, b| a.hlc.cmp(&b.hlc));
                self.home = latest.and_then(|h| h.place.clone());
                self.home_since = latest.map(|h| h.recorded_at);
            }
            Event::SnoozeAllStarted { snooze } => {
                if self
                    .snooze_alls
                    .iter()
                    .any(|r| r.event_id == stored.event_id)
                {
                    return;
                }
                self.snooze_alls.push(SnoozeAllRecord {
                    event_id: stored.event_id.clone(),
                    snooze: snooze.clone(),
                    by: stored.author.clone(),
                    device_id: stored.device_id.clone(),
                    recorded_at: stored.recorded_at,
                    ended: self.snooze_all_ends.get(&snooze.id).copied(),
                });
            }
            Event::SnoozeAllEnded { snooze_id } => {
                // Of several ends, the first counts.
                let at = self
                    .snooze_all_ends
                    .entry(snooze_id.clone())
                    .and_modify(|e| *e = (*e).min(stored.recorded_at))
                    .or_insert(stored.recorded_at);
                let at = *at;
                for r in self
                    .snooze_alls
                    .iter_mut()
                    .filter(|r| &r.snooze.id == snooze_id)
                {
                    r.ended = Some(at);
                }
            }
            Event::QuietHoursSet { hlc, rules } => {
                if self
                    .quiet_versions
                    .iter()
                    .any(|q| q.event_id == stored.event_id)
                {
                    return;
                }
                let hlc = hlc.clamped(stored.recorded_at);
                if hlc > self.latest_hlc {
                    self.latest_hlc = hlc.clone();
                }
                self.quiet_versions.push(QuietHoursVersion {
                    event_id: stored.event_id.clone(),
                    hlc,
                    rules: rules.clone(),
                    recorded_at: stored.recorded_at,
                });
                // Of several, the latest clock counts, whatever order they
                // arrive in.
                self.quiet_hours = self
                    .quiet_versions
                    .iter()
                    .max_by(|a, b| a.hlc.cmp(&b.hlc))
                    .map(|q| q.rules.clone())
                    .unwrap_or_default();
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
            Event::DevicePortable { portable } => {
                self.device_portable
                    .insert(stored.device_id.clone(), *portable);
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

    /// Adds an occurrence a reminder fired, unless it is one the reminder
    /// already has (ADR 0001): a one-off has only one, and for a repeating
    /// reminder the older open ones are missed.
    fn insert_occurrence(
        &mut self,
        occurrence_id: &String,
        reminder_id: &String,
        scheduled_at: i64,
        fired_at: i64,
    ) {
        if self.aliases.contains_key(occurrence_id) {
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
                scheduled_at,
                fired_at,
                closing: None,
                records: Vec::new(),
                undos: Vec::new(),
                unfired: false,
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
        if let (Some((until, at)), Some(o)) = (ahead, self.occurrences.get_mut(occurrence_id)) {
            o.snoozed_until = Some(until);
            o.snoozed_at = Some(at);
        }
        if !one_off {
            self.expire_superseded(reminder_id);
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

    /// The snooze-all or quiet hours stretch holding an occurrence of
    /// `priority` in `list_id`, scheduled at `scheduled_at` and open at `now`,
    /// if one does. Of several, the one that ends last. Only the personal
    /// list's state has any (ADR 0013).
    pub fn hold_on(
        &self,
        zone: &TimeZone,
        list_id: &str,
        priority: Priority,
        scheduled_at: i64,
        now: i64,
    ) -> Option<Hold> {
        let all = self
            .snooze_alls
            .iter()
            .filter(|r| r.snooze.reaches(list_id, priority))
            .filter(|r| {
                let until = r.effective_until();
                r.snooze.from <= now && now < until && scheduled_at < until
            })
            .map(|r| Hold {
                since: r.snooze.from,
                until: r.effective_until(),
                source: Source::SnoozeAll,
            })
            .max_by_key(|h| h.until);
        let quiet = quiet::quiet_hold(&self.quiet_hours, zone, list_id, priority, now);
        match (all, quiet) {
            (Some(a), Some(q)) => Some(if q.until > a.until { q } else { a }),
            (a, q) => a.or(q),
        }
    }

    /// The snoozes the settings put on an occurrence, as the history shows
    /// them: one for each snooze-all or stretch of quiet hours it was open
    /// during, oldest first. `closed_at` is when it was closed, if it has
    /// been. Quiet hours are read as they stand now.
    pub fn holds_over(
        &self,
        zone: &TimeZone,
        list_id: &str,
        priority: Priority,
        scheduled_at: i64,
        closed_at: Option<i64>,
        now: i64,
    ) -> Vec<SnoozeView> {
        let open_at = |from: i64| closed_at.is_none_or(|c| c > from);
        // How a hold of `until` that began for this occurrence at `set_at`
        // ended, if it has.
        let ending = |until: i64, early: Option<i64>| -> (Option<i64>, Option<SnoozeEnd>) {
            match (closed_at, early) {
                (Some(c), _) if c < until && early.is_none_or(|e| c <= e) => {
                    (Some(c), Some(SnoozeEnd::Closed))
                }
                (_, Some(e)) if e < until => (Some(e), Some(SnoozeEnd::Ended)),
                _ if until <= now => (Some(until), Some(SnoozeEnd::Elapsed)),
                _ => (None, None),
            }
        };
        let mut out = Vec::new();
        for r in self
            .snooze_alls
            .iter()
            .filter(|r| r.snooze.reaches(list_id, priority))
        {
            let s = &r.snooze;
            let until = r.effective_until();
            if s.from > now || scheduled_at >= until || !open_at(s.from) {
                continue;
            }
            let (ended_at, ended) = ending(s.until, r.ended);
            out.push(SnoozeView {
                occurrence_id: String::new(),
                set_at: s.from.max(scheduled_at),
                until: s.until,
                ahead: false,
                via: Some(Source::SnoozeAll),
                ended_at,
                ended,
            });
        }
        let horizon = closed_at.unwrap_or(now).min(now);
        let mut seen = BTreeSet::new();
        for rule in self
            .quiet_hours
            .iter()
            .filter(|q| q.reaches(list_id, priority))
        {
            for (start, end) in rule.stretches(zone, scheduled_at, horizon.saturating_add(1), 400) {
                if start > now
                    || scheduled_at >= end
                    || !open_at(start)
                    || !seen.insert((start, end))
                {
                    continue;
                }
                let (ended_at, ended) = ending(end, None);
                out.push(SnoozeView {
                    occurrence_id: String::new(),
                    set_at: start.max(scheduled_at),
                    until: end,
                    ahead: false,
                    via: Some(Source::QuietHours),
                    ended_at,
                    ended,
                });
            }
        }
        out.sort_by_key(|v| (v.set_at, v.until));
        out
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
                    via: None,
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

    /// Closes an occurrence. Closings pile up as records and
    /// [`State::settle`] decides which counts, whatever order they arrive in.
    fn close(
        &mut self,
        occurrence_id: &str,
        kind: ClosingKind,
        at: i64,
        note: Option<String>,
        stored: &StoredEvent,
    ) {
        let id = self.resolve(occurrence_id).to_string();
        self.add_record(&id, Closing::new(kind, at, note, stored));
    }

    /// Adds a closing to an occurrence's records, once, and settles it.
    fn add_record(&mut self, occurrence_id: &str, closing: Closing) {
        let id = self.resolve(occurrence_id).to_string();
        let Some(o) = self.occurrences.get_mut(&id) else {
            return;
        };
        if o.records.iter().any(|c| c.event_id == closing.event_id) {
            return;
        }
        o.records.push(closing);
        self.settle(&id);
    }

    /// Works out how an occurrence stands from everything done to it. The
    /// rules for devices that disagree, which hold whatever order events
    /// arrive in:
    ///
    /// - a correction or an undo takes the place of the closings its author
    ///   had seen, and only those: a closing another device made meanwhile
    ///   stands. Undoing a correction brings back what it corrected.
    /// - of the closings that stand, a completion beats a skip, which beats a
    ///   miss; of the same kind the first in the stream counts.
    /// - a completion over another kind is recorded, for the loser to be told.
    /// - with no closing standing it is open, or expected again if it was
    ///   closed ahead of its time and undone.
    fn settle(&mut self, occurrence_id: &str) {
        let Some(o) = self.occurrences.get(occurrence_id) else {
            return;
        };
        let superseded = superseded(&o.records, &o.undos);
        let mut live: Vec<&Closing> = o
            .records
            .iter()
            .filter(|c| !superseded.contains(c.event_id.as_str()))
            .collect();
        // Stable: of one kind, the one the stream has first counts (the
        // server numbers it, so every device agrees).
        live.sort_by_key(|c| std::cmp::Reverse(c.kind));
        let winner = live.first().map(|c| (*c).clone());
        let reconciled: Vec<Reconciliation> = match &winner {
            Some(w) if w.kind == ClosingKind::Completed => live[1..]
                .iter()
                .filter(|c| c.kind != ClosingKind::Completed)
                .map(|c| Reconciliation {
                    id: c.event_id.clone(),
                    occurrence_id: occurrence_id.to_string(),
                    device_id: c.device_id.clone(),
                    by: c.by.clone(),
                    lost: c.kind,
                    lost_at: c.at,
                })
                .collect(),
            _ => Vec::new(),
        };
        let expected_again = winner.is_none()
            && o.is_early()
            && o.undos.iter().any(|u| {
                !superseded.contains(u.event_id.as_str()) && u.outcome == UndoOutcome::Expected
            });
        let was_closed = o.closing.is_some();
        let reminder_id = o.reminder_id.clone();
        self.reconciliations
            .retain(|r| r.occurrence_id != occurrence_id);
        self.reconciliations.extend(reconciled);
        let Some(o) = self.occurrences.get_mut(occurrence_id) else {
            return;
        };
        o.unfired = expected_again;
        let closed_at = winner.as_ref().map(|c| c.at);
        if let (Some(at), false) = (closed_at, was_closed) {
            o.snoozed_until = None;
            o.snoozed_at = None;
            o.acknowledged = false;
            o.acknowledged_at = None;
            o.closing = winner;
            self.end_snoozes(occurrence_id, at);
            return;
        }
        o.closing = winner;
        // Reopened: a newer occurrence of a repeating reminder takes its place.
        if was_closed
            && o.closing.is_none()
            && self
                .reminders
                .get(&reminder_id)
                .is_some_and(|r| r.repeats())
        {
            self.expire_superseded(&reminder_id);
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
            .filter(|o| o.reminder_id == reminder_id && !o.unfired)
            .map(|o| (o.scheduled_at, o.id.clone()))
            .max()
        else {
            return;
        };
        let older: Vec<(String, i64)> = self
            .occurrences
            .values()
            .filter(|o| o.reminder_id == reminder_id && o.id != newest.1 && o.is_open())
            .map(|o| (o.id.clone(), newest.0.max(o.scheduled_at)))
            .collect();
        for (id, at) in older {
            self.add_record(
                &id,
                Closing {
                    kind: ClosingKind::Missed,
                    by: String::new(),
                    device_id: String::new(),
                    at,
                    note: None,
                    event_id: format!("expired:{id}"),
                    recorded_at: at,
                    replaces: Vec::new(),
                    paused: None,
                },
            );
        }
    }

    /// Everything done to how an occurrence is closed, oldest first: each
    /// closing, correction and undo, with the ones that were taken back
    /// marked. Nothing is ever removed.
    pub fn history_of(&self, occurrence_id: &str) -> Vec<HistoryEntry> {
        let Some(o) = self.occurrences.get(self.resolve(occurrence_id)) else {
            return Vec::new();
        };
        let gone = superseded(&o.records, &o.undos);
        let mut v: Vec<HistoryEntry> = o
            .records
            .iter()
            .map(|c| HistoryEntry {
                event_id: c.event_id.clone(),
                what: match c.kind {
                    ClosingKind::Completed => HistoryWhat::Completed,
                    ClosingKind::Skipped => HistoryWhat::Skipped,
                    ClosingKind::Missed => HistoryWhat::Missed,
                },
                at: Some(c.at),
                note: c.note.clone(),
                by: c.by.clone(),
                device_id: c.device_id.clone(),
                recorded_at: c.recorded_at,
                superseded: gone.contains(c.event_id.as_str()),
                replaces: c.replaces.clone(),
                paused: c.paused,
            })
            .chain(o.undos.iter().map(|u| HistoryEntry {
                event_id: u.event_id.clone(),
                what: match u.outcome {
                    UndoOutcome::Expected => HistoryWhat::Expected,
                    _ => HistoryWhat::Reopened,
                },
                at: None,
                note: None,
                by: u.by.clone(),
                device_id: u.device_id.clone(),
                recorded_at: u.recorded_at,
                superseded: gone.contains(u.event_id.as_str()),
                replaces: u.replaces.clone(),
                paused: None,
            }))
            .collect();
        v.sort_by(|a, b| (a.recorded_at, &a.event_id).cmp(&(b.recorded_at, &b.event_id)));
        v
    }

    /// What a correction or an undo made now takes the place of: the
    /// closings that count, and everything they in turn replaced, so that
    /// the occurrence is changed as a whole and not one step back. A closing
    /// another device makes meanwhile isn't among them, and stands.
    pub fn standing_closings(&self, occurrence_id: &str) -> Vec<String> {
        let Some(o) = self.occurrences.get(self.resolve(occurrence_id)) else {
            return Vec::new();
        };
        let gone = superseded(&o.records, &o.undos);
        let mut out: Vec<String> = Vec::new();
        let mut todo: Vec<&str> = o
            .records
            .iter()
            .filter(|c| !gone.contains(c.event_id.as_str()))
            .map(|c| c.event_id.as_str())
            .collect();
        while let Some(id) = todo.pop() {
            if out.iter().any(|o| o == id) {
                continue;
            }
            out.push(id.to_string());
            if let Some(c) = o.records.iter().find(|c| c.event_id == id) {
                todo.extend(c.replaces.iter().map(String::as_str));
            }
        }
        out.sort();
        out
    }

    /// Notes given when skipping, most recent first and each once, up to
    /// `limit`: offered again the next time.
    pub fn recent_skip_notes(&self, limit: usize) -> Vec<(i64, String)> {
        let mut v: Vec<(i64, String)> = self
            .occurrences
            .values()
            .flat_map(|o| &o.records)
            .filter(|c| c.kind == ClosingKind::Skipped)
            .filter_map(|c| Some((c.recorded_at, c.note.clone().filter(|n| !n.is_empty())?)))
            .collect();
        v.sort_by(|a, b| b.cmp(a));
        let mut seen = BTreeSet::new();
        v.retain(|(_, n)| seen.insert(n.clone()));
        v.truncate(limit);
        v
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
            .filter(|o| o.reminder_id == reminder_id && !o.unfired)
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
            Setting::Pause,
            Setting::SunEvents,
            Setting::Conditions,
        ] {
            let latest = self
                .versions
                .get(&(reminder_id.to_string(), setting))
                .and_then(|v| v.iter().max_by(|a, b| a.hlc.cmp(&b.hlc)));
            if matches!(
                setting,
                Setting::Schedules | Setting::Zone | Setting::SunEvents | Setting::Conditions
            ) {
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
                Some(Change::Pause(p)) => r.pause = p,
                Some(Change::SunEvents(v)) => r.suns = v,
                Some(Change::Conditions(v)) => r.conditions = v,
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

    /// What puts the reminder under a pause at `t`, if anything: its own
    /// pause, or else its list's. Instances that fall in a pause are skipped
    /// ([`crate::Core::tick`]) and event triggers don't fire in it.
    pub fn pause_at(&self, r: &Reminder, t: i64) -> Option<PauseCause> {
        let own = r.pause.filter(|p| p.covers(t)).map(|p| PauseCause {
            until: p.until,
            list: false,
        });
        own.or_else(|| {
            self.list_pause.filter(|p| p.covers(t)).map(|p| PauseCause {
                until: p.until,
                list: true,
            })
        })
    }

    /// Whether the reminder is paused at `now`.
    pub fn is_paused(&self, reminder_id: &str, now: i64) -> bool {
        self.reminders
            .get(reminder_id)
            .is_some_and(|r| self.pause_at(r, now).is_some())
    }

    /// Whether the reminder has fired yet (its one occurrence exists).
    pub(crate) fn has_fired(&self, reminder_id: &str) -> bool {
        self.occurrences
            .values()
            .any(|o| o.reminder_id == reminder_id && !o.unfired)
    }

    /// Whether an occurrence exists and has really fired or been closed
    /// early: not one that was closed ahead of its time and undone.
    pub(crate) fn is_fired(&self, occurrence_id: &str) -> bool {
        self.occurrences
            .get(self.resolve(occurrence_id))
            .is_some_and(|o| !o.unfired)
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
                    if mine.is_early() && !o.is_early() {
                        mine.fired_at = o.fired_at;
                    }
                    for c in &o.records {
                        if !mine.records.iter().any(|m| m.event_id == c.event_id) {
                            mine.records.push(c.clone());
                        }
                    }
                    for u in &o.undos {
                        if !mine.undos.iter().any(|m| m.event_id == u.event_id) {
                            mine.undos.push(u.clone());
                        }
                    }
                }
            }
            self.settle(&o.id);
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
