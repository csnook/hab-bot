use std::collections::BTreeMap;
use std::path::Path;

use jiff::tz::TimeZone;
use serde::Serialize;
use uuid::Uuid;

use crate::countdown::Countdown;
use crate::event::{
    Change, Event, Outgoing, Payload, Setting, StoredEvent, FORMAT_VERSION, UPDATE_NOTICE,
};
use crate::hlc::Hlc;
use crate::priority::{AlertStyle, Priority};
use crate::schedule::{self, Schedule};
use crate::state::{
    last_chance_at, ClosingKind, DueItem, Reminder, SnoozeView, State, UpcomingItem,
};
use crate::store::Store;
use crate::{Error, Result};

/// An occurrence that has just fired, for the platform to alert about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fired {
    pub occurrence_id: String,
    pub reminder_id: String,
    pub title: String,
}

/// What the window shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// Every open occurrence, due or overdue. The Inbox splits them (`Core::inbox`).
    pub due: Vec<DueItem>,
    /// Reminders that haven't fired yet.
    pub upcoming: Vec<UpcomingItem>,
    /// Countdown reminders and when each fires next.
    pub countdowns: Vec<CountdownItem>,
    /// Set while the list holds changes from a newer app, which this one keeps
    /// without applying.
    pub update_notice: Option<String>,
    /// Other devices of this user that signed in, not yet dismissed.
    pub sign_in_notices: Vec<SignInNotice>,
    /// Actions of this user's devices that lost to a completion on another.
    pub reconciliations: Vec<ReconciliationNotice>,
    /// Things the server told this user's devices, such as failed sign-ins,
    /// not yet dismissed.
    pub security_notices: Vec<SecurityNotice>,
    /// Things the app did to this device's data that the user should know,
    /// such as settings giving way to the account's, not yet dismissed.
    pub notices: Vec<DeviceNotice>,
}

/// "Your quiet hours here were replaced by your account's." Kept on this
/// device until the user dismisses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceNotice {
    /// Names the notice, for dismissing it.
    pub id: String,
    pub text: String,
}

const SETTINGS_GAVE_WAY: &str = "settings-gave-way";
const SETTINGS_GAVE_WAY_TEXT: &str =
    "This device's own personal settings, such as quiet hours, were replaced by your account's. \
     Settings that belong to this device alone were kept.";
const PERSONAL_SETTING: &str = "personal_setting:";
const DEVICE_SETTING: &str = "device_setting:";

/// An open occurrence as the alarm window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OccurrenceView {
    pub occurrence_id: String,
    pub title: String,
    /// The list's name; `None` for the personal list.
    pub list_name: Option<String>,
    pub priority: Priority,
    /// When it was due.
    pub scheduled_at: i64,
    pub overdue_at: i64,
    pub snoozed_until: Option<i64>,
    pub acknowledged_at: Option<i64>,
    /// When it is missed for want of action, if it has such an expiry.
    pub expires_at: Option<i64>,
}

/// One of the snooze menu's timed choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnoozeKind {
    /// The priority's current interval.
    Interval,
    /// One hour (left out when the interval is one hour).
    Hour,
    /// 08:00 the next day.
    TomorrowMorning,
}

/// A choice in the snooze menu and when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SnoozeOption {
    pub kind: SnoozeKind,
    pub until: i64,
    /// The length of an interval or hour, in seconds.
    pub seconds: Option<i64>,
    /// A last-chance alert would come inside this snooze, before the expiry.
    pub last_chance: bool,
}

/// What the snooze menu shows for an open or expected occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnoozePicker {
    /// The priority's interval, 1 hour and tomorrow morning. "Until a time"
    /// and "pick a time" take a time of the user's choosing, which the menu
    /// checks against `expires_at` (see [`last_chance_at`]).
    pub options: Vec<SnoozeOption>,
    /// "Expires at 23:59": a known expiry, if the occurrence has one.
    pub expires_at: Option<i64>,
    /// When the last-chance alert comes before it.
    pub last_chance_at: Option<i64>,
    /// An expected occurrence: the choices count from its time, which is
    /// `from`; it still fires then, quietly.
    pub ahead: bool,
    /// What the choices count from: now, or an expected occurrence's time.
    pub from: i64,
}

/// The hour tomorrow morning means, in the device's time zone.
const TOMORROW_MORNING_HOUR: i8 = 8;

/// A reminder list this device holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListInfo {
    pub id: String,
    /// What the list is called. The personal list is not named.
    pub name: Option<String>,
    /// The account's personal list, which also holds its settings.
    pub personal: bool,
}

/// "5 failed sign-ins to your account", from the server. The server has no
/// device to author an event, so these are not in the list's stream: each
/// device keeps what the server told it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecurityNotice {
    /// Names the notice, for dismissing it.
    pub id: String,
    pub text: String,
    pub count: u32,
    /// When the server raised it, in Unix seconds.
    pub at: i64,
}

const SERVER_NOTICE: &str = "server_notice:";
const FAILED_SIGN_INS: &str = "failed_sign_ins";

/// "Your phone skipped “Bins”. It counts as completed."
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconciliationNotice {
    /// Names the notice, for dismissing it.
    pub id: String,
    pub occurrence_id: String,
    /// The device whose action lost.
    pub device_id: String,
    pub device_name: Option<String>,
    pub text: String,
}

/// A countdown reminder, for the list of what is counting down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CountdownItem {
    pub reminder_id: String,
    pub title: String,
    pub countdown: Countdown,
    /// When it fires next. `None` while an occurrence is open: it restarts
    /// when that closes.
    pub next_at: Option<i64>,
}

/// New values for a reminder's settings; `None` leaves a setting alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditReminder {
    pub title: Option<String>,
    pub fire_at: Option<i64>,
    pub note: Option<String>,
    /// All the schedule triggers, replacing the reminder's.
    pub schedules: Option<Vec<Schedule>>,
    /// Pins the reminder to a time zone, or with `Some(None)` makes it
    /// floating.
    pub zone: Option<Option<String>>,
    pub priority: Option<Priority>,
    /// Overrides the priority's overdue time with a number of seconds after
    /// the scheduled time, or with `Some(None)` goes back to following it.
    pub overdue: Option<Option<i64>>,
    /// Seconds after the scheduled time that an open occurrence is missed,
    /// or with `Some(None)` no such expiry.
    pub expiry: Option<Option<i64>>,
    /// A new countdown, for a countdown reminder.
    pub countdown: Option<Countdown>,
}

/// An occurrence predicted to come due: it becomes an occurrence only if the
/// reminder fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExpectedItem {
    pub reminder_id: String,
    pub title: String,
    pub scheduled_at: i64,
    /// Snoozed ahead of time until then: it fires at its time, quietly.
    pub snoozed_until: Option<i64>,
    /// When it would be missed for want of action, if it has such an expiry.
    pub expires_at: Option<i64>,
}

/// A closed occurrence for the Inbox's Earlier today section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EarlierItem {
    pub occurrence_id: String,
    pub title: String,
    pub scheduled_at: i64,
    pub closed_at: i64,
    pub kind: ClosingKind,
}

/// The Inbox's sections about the rest of today and what's behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inbox {
    /// Open occurrences past their overdue time: highest priority first,
    /// then the longest overdue.
    pub overdue: Vec<DueItem>,
    /// Open occurrences not yet overdue, oldest firing first.
    pub due: Vec<DueItem>,
    /// Expected occurrences from now to the end of today, earliest first.
    pub later_today: Vec<ExpectedItem>,
    /// Occurrences closed today, including missed ones, latest first.
    pub earlier_today: Vec<EarlierItem>,
}

/// Instances kept when a device was away long enough to pass many: the most
/// recent ones. Older ones are not recorded.
const MAX_LATE_INSTANCES: usize = 50;
/// How many instances are looked at in one go.
const MAX_INSTANCES: usize = 5_000;
const DEVICE_ZONE: &str = "time_zone";
const DAY: i64 = 86_400;

/// "New device signed in: <name>, just now. Not you? Remove it".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignInNotice {
    /// Names the notice, for dismissing it.
    pub id: String,
    pub device_id: String,
    pub device_name: String,
    /// When the device signed in, in Unix seconds.
    pub at: i64,
}

/// The core for one device: its storage, its user, and the state of the
/// personal list and of any other lists it holds, such as the one a
/// standalone device's reminders became when it signed in to an account.
pub struct Core {
    store: Store,
    /// The personal list's state.
    state: State,
    /// The state of every other list this device holds, by id.
    others: BTreeMap<String, State>,
    list_id: String,
    device_id: String,
    user_id: String,
    /// Events from a newer app are being kept unapplied.
    holding_newer: bool,
}

impl Core {
    pub fn open(path: &Path) -> Result<Self> {
        Self::from_store(Store::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_store(Store::open_in_memory()?)
    }

    /// Builds the current state by applying the personal list's stream.
    pub fn from_store(store: Store) -> Result<Self> {
        let new_id = || Uuid::new_v4().to_string();
        let list_id = store.meta_or_init("personal_list_id", new_id)?;
        let device_id = store.meta_or_init("device_id", new_id)?;
        let user_id = store.meta_or_init("user_id", new_id)?;
        let mut core = Core {
            store,
            state: State::default(),
            others: BTreeMap::new(),
            list_id,
            device_id,
            user_id,
            holding_newer: false,
        };
        core.rebuild()?;
        Ok(core)
    }

    /// Builds the state again from the stream: the server's numbered events in
    /// order, then this device's unsent ones on top.
    fn rebuild(&mut self) -> Result<()> {
        let build = |store: &Store, list_id: &str| -> Result<State> {
            let mut state = State::default();
            for e in store.stream(list_id)? {
                state.apply(&e);
            }
            Ok(state)
        };
        self.state = build(&self.store, &self.list_id)?;
        let mut others = BTreeMap::new();
        let mut holding_newer = !self.store.held(&self.list_id)?.is_empty();
        for id in self.store.list_ids()? {
            if id == self.list_id {
                continue;
            }
            holding_newer |= !self.store.held(&id)?.is_empty();
            let state = build(&self.store, &id)?;
            others.insert(id, state);
        }
        self.others = others;
        self.holding_newer = holding_newer;
        Ok(())
    }

    /// The state of a list this device holds.
    pub fn state_of(&self, list_id: &str) -> Option<&State> {
        if list_id == self.list_id {
            Some(&self.state)
        } else {
            self.others.get(list_id)
        }
    }

    /// Every list this device holds: the personal list first, then the
    /// others by id. A list the device has no events of yet is not held.
    pub fn lists(&self) -> Vec<ListInfo> {
        let mut v = vec![ListInfo {
            id: self.list_id.clone(),
            name: None,
            personal: true,
        }];
        v.extend(self.others.iter().map(|(id, s)| ListInfo {
            id: id.clone(),
            name: s.list_name.clone(),
            personal: false,
        }));
        v
    }

    /// Every state, personal list first.
    fn states(&self) -> impl Iterator<Item = (&str, &State)> {
        std::iter::once((self.list_id.as_str(), &self.state))
            .chain(self.others.iter().map(|(id, s)| (id.as_str(), s)))
    }

    fn state_mut(&mut self, list_id: &str) -> &mut State {
        if list_id == self.list_id {
            &mut self.state
        } else {
            self.others.entry(list_id.to_string()).or_default()
        }
    }

    pub fn personal_list_id(&self) -> &str {
        &self.list_id
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Whether this device has joined a server.
    pub fn is_joined(&self) -> Result<bool> {
        self.store.joined()
    }

    /// This device has joined a server as `user_id`, as device `device_id`
    /// there. Everything it made while standalone is now that user's and
    /// that device's, still unsent: the sync layer uploads it, history
    /// included, and the server numbers it.
    pub fn join(&mut self, user_id: &str, device_id: &str) -> Result<()> {
        self.store.adopt(user_id, device_id)?;
        self.user_id = user_id.to_string();
        self.device_id = device_id.to_string();
        // The standalone numbers were this device's own; the server's replace them.
        self.store.forget_local_numbers(&self.list_id)?;
        self.rebuild()
    }

    /// Use the account's personal list, whose id the server gave this device,
    /// in place of the one it made for itself. Only for a device whose own
    /// list is still empty, such as one just added to an account.
    pub fn use_personal_list(&mut self, list_id: &str) -> Result<()> {
        if list_id == self.list_id {
            return Ok(());
        }
        if !self.store.stream(&self.list_id)?.is_empty() {
            return Err(Error::BadEvent("this device already has its own list"));
        }
        self.store.set_meta("personal_list_id", list_id)?;
        self.list_id = list_id.to_string();
        self.rebuild()
    }

    /// This device has signed in to an existing account, whose personal list
    /// has the id `list_id`. The account's personal list becomes this
    /// device's. What the device made while standalone is never merged into
    /// it: if there is any, it stays a list of its own, with its reminders,
    /// occurrences and history exactly as they were, and is named `name`
    /// (the device's). It is uploaded and synced like any other list.
    ///
    /// The device's standalone personal settings give way to the account's,
    /// and a notice says so; settings that belong to this device alone stay.
    /// Safe to run again after a failure part way: each step checks first.
    pub fn link_account(&mut self, list_id: &str, name: &str, now: i64) -> Result<()> {
        if list_id == self.list_id {
            return Ok(());
        }
        let standalone = self.list_id.clone();
        let has_events = self.store.list_ids()?.contains(&standalone);
        if has_events {
            // Named first, so a list that arrives is never unnamed.
            if self.state.list_name.is_none() {
                let name = name.trim();
                let name = if name.is_empty() { "This device" } else { name };
                let stored = self.store.append(
                    &standalone,
                    &self.device_id,
                    &self.user_id,
                    now,
                    Event::ListNamed {
                        name: name.to_string(),
                    },
                    Uuid::new_v4().to_string(),
                )?;
                self.state.apply(&stored);
            }
        }
        let had_settings = !self.store.meta_prefix(PERSONAL_SETTING)?.is_empty();
        self.store.set_meta("personal_list_id", list_id)?;
        self.list_id = list_id.to_string();
        if had_settings {
            self.store.delete_meta_prefix(PERSONAL_SETTING)?;
            self.store
                .set_meta(&format!("notice:{SETTINGS_GAVE_WAY}"), "1")?;
        }
        self.rebuild()
    }

    /// A setting that belongs to the user, such as quiet hours. A standalone
    /// device keeps these itself; once it is on an account, the account's
    /// replace them ([`Self::link_account`]).
    pub fn set_personal_setting(&self, key: &str, value: &str) -> Result<()> {
        self.store
            .set_meta(&format!("{PERSONAL_SETTING}{key}"), value)
    }

    pub fn personal_setting(&self, key: &str) -> Result<Option<String>> {
        self.store.meta(&format!("{PERSONAL_SETTING}{key}"))
    }

    /// A setting that stays on this device, such as its loudest alert style.
    /// Signing in to an account leaves these alone.
    pub fn set_device_setting(&self, key: &str, value: &str) -> Result<()> {
        self.store
            .set_meta(&format!("{DEVICE_SETTING}{key}"), value)
    }

    pub fn device_setting(&self, key: &str) -> Result<Option<String>> {
        self.store.meta(&format!("{DEVICE_SETTING}{key}"))
    }

    fn device_notices(&self) -> Vec<DeviceNotice> {
        let id = SETTINGS_GAVE_WAY;
        let shown = matches!(self.store.meta(&format!("notice:{id}")), Ok(Some(_)));
        let dismissed = matches!(self.store.meta(&format!("dismissed:{id}")), Ok(Some(_)));
        if shown && !dismissed {
            vec![DeviceNotice {
                id: id.to_string(),
                text: SETTINGS_GAVE_WAY_TEXT.to_string(),
            }]
        } else {
            Vec::new()
        }
    }

    /// The events the server hasn't numbered, in the order they were made:
    /// the personal list's, then each other list's.
    pub fn unsent(&self) -> Result<Vec<Outgoing>> {
        let mut out = Vec::new();
        let ids: Vec<String> = self.states().map(|(id, _)| id.to_string()).collect();
        for list_id in ids {
            for row in self.store.unsent(&list_id)? {
                let payload = Payload {
                    author: row.author,
                    recorded_at: row.recorded_at,
                    event: serde_json::from_slice(&row.body)?,
                };
                out.push(Outgoing {
                    list_id: list_id.clone(),
                    event_id: row.event_id,
                    format: row.format,
                    recorded_at: payload.recorded_at,
                    payload: serde_json::to_vec(&payload)?,
                });
            }
        }
        Ok(out)
    }

    /// The server numbered one of this device's events.
    pub fn mark_sent(&mut self, event_id: &str, seq: i64) -> Result<()> {
        self.store.set_seq(event_id, seq)?;
        self.rebuild()
    }

    /// An event the server numbered, decrypted and verified by the sync layer.
    /// One in a newer format is kept without being applied. Returns whether it
    /// was new to this device.
    pub fn receive(
        &mut self,
        list_id: &str,
        seq: i64,
        event_id: &str,
        device_id: &str,
        format: u32,
        payload: &[u8],
    ) -> Result<bool> {
        let p: Payload =
            serde_json::from_slice(payload).map_err(|_| Error::BadEvent("not a payload"))?;
        if format <= FORMAT_VERSION && serde_json::from_value::<Event>(p.event.clone()).is_err() {
            return Err(Error::BadEvent("not an event"));
        }
        let inserted = self.store.insert_numbered(
            list_id,
            seq,
            event_id,
            device_id,
            &p.author,
            p.recorded_at,
            format,
            &serde_json::to_vec(&p.event)?,
        )?;
        self.rebuild()?;
        Ok(inserted)
    }

    /// Ids of events kept without being applied, because a newer app made them.
    pub fn held_events(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for (id, _) in self.states() {
            out.extend(self.store.held(id)?.into_iter().map(|h| h.event_id));
        }
        Ok(out)
    }

    /// The highest server number this device has downloaded up to, in the
    /// personal list.
    pub fn cursor(&self) -> Result<i64> {
        self.cursor_of(&self.list_id)
    }

    pub fn set_cursor(&self, seq: i64) -> Result<()> {
        self.set_cursor_of(&self.list_id, seq)
    }

    /// The same for any list this device holds.
    pub fn cursor_of(&self, list_id: &str) -> Result<i64> {
        Ok(self
            .store
            .meta(&format!("cursor:{list_id}"))?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    pub fn set_cursor_of(&self, list_id: &str, seq: i64) -> Result<()> {
        self.store
            .set_meta(&format!("cursor:{list_id}"), &seq.to_string())
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    fn record(&mut self, now: i64, event: Event) -> Result<StoredEvent> {
        let list_id = self.list_id.clone();
        self.record_in(&list_id, now, event)
    }

    fn record_in(&mut self, list_id: &str, now: i64, event: Event) -> Result<StoredEvent> {
        let stored = self.store.append(
            list_id,
            &self.device_id,
            &self.user_id,
            now,
            event,
            Uuid::new_v4().to_string(),
        )?;
        self.state_mut(list_id).apply(&stored);
        Ok(stored)
    }

    /// The list a reminder is in.
    fn list_of_reminder(&self, reminder_id: &str) -> Result<String> {
        self.states()
            .find(|(_, s)| s.reminders.contains_key(reminder_id))
            .map(|(id, _)| id.to_string())
            .ok_or_else(|| Error::NoReminder(reminder_id.to_string()))
    }

    /// Creates a one-off reminder in the personal list, to fire at `fire_at`.
    pub fn create_reminder(&mut self, title: &str, fire_at: i64, now: i64) -> Result<String> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::EmptyTitle);
        }
        let reminder_id = Uuid::new_v4().to_string();
        self.record(
            now,
            Event::ReminderCreated {
                reminder_id: reminder_id.clone(),
                title: title.to_string(),
                fire_at,
            },
        )?;
        Ok(reminder_id)
    }

    /// Creates a reminder in the personal list that repeats on `schedules`
    /// (several of them mean several times). `zone` pins it to a named time
    /// zone; `None` makes it floating, firing at the same local time wherever
    /// the device is. Nothing fires for instants before `now`.
    pub fn create_recurring_reminder(
        &mut self,
        title: &str,
        schedules: Vec<Schedule>,
        zone: Option<&str>,
        now: i64,
    ) -> Result<String> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::EmptyTitle);
        }
        check_schedules(&schedules, zone)?;
        let reminder_id = Uuid::new_v4().to_string();
        self.record(
            now,
            Event::RecurringReminderCreated {
                reminder_id: reminder_id.clone(),
                title: title.to_string(),
                schedules,
                zone: zone.map(str::to_string),
            },
        )?;
        Ok(reminder_id)
    }

    /// Creates a reminder in the personal list that fires `countdown` after
    /// its last occurrence closed. `last_done` is when it was last done, which
    /// starts the first countdown (the caller defaults it to now); `None`
    /// means never, and it fires at once. `zone` pins a countdown with a time
    /// of day to a time zone; `None` follows the device's.
    pub fn create_countdown_reminder(
        &mut self,
        title: &str,
        countdown: Countdown,
        zone: Option<&str>,
        last_done: Option<i64>,
        now: i64,
    ) -> Result<String> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::EmptyTitle);
        }
        countdown.validate().map_err(Error::BadCountdown)?;
        check_schedules(&[], zone)?;
        if last_done.is_some_and(|t| t > now) {
            return Err(Error::InTheFuture);
        }
        let reminder_id = Uuid::new_v4().to_string();
        self.record(
            now,
            Event::CountdownReminderCreated {
                reminder_id: reminder_id.clone(),
                title: title.to_string(),
                countdown,
                zone: zone.map(str::to_string),
                last_done,
            },
        )?;
        Ok(reminder_id)
    }

    /// The time zone floating reminders follow on this device. Until the app
    /// says (from the system's setting), UTC.
    pub fn device_zone(&self) -> String {
        self.device_setting(DEVICE_ZONE)
            .ok()
            .flatten()
            .filter(|z| schedule::zone(z).is_some())
            .unwrap_or_else(|| "UTC".to_string())
    }

    /// Sets the time zone this device is in. The device's own: it is not
    /// synced, and a reminder pinned to a zone ignores it.
    pub fn set_device_zone(&self, name: &str) -> Result<()> {
        if schedule::zone(name).is_none() {
            return Err(Error::BadZone(name.to_string()));
        }
        self.set_device_setting(DEVICE_ZONE, name)
    }

    /// Follows the system's time zone, which changes when the user travels.
    pub fn use_system_zone(&self) -> Result<()> {
        match schedule::system_zone_name() {
            Some(name) if schedule::zone(&name).is_some() => self.set_device_zone(&name),
            _ => Ok(()),
        }
    }

    fn zone_of(&self, r: &Reminder) -> TimeZone {
        r.zone
            .as_deref()
            .and_then(schedule::zone)
            .or_else(|| schedule::zone(&self.device_zone()))
            .unwrap_or(TimeZone::UTC)
    }

    /// The instants a repeating reminder's schedules have after `after` and
    /// up to `until`, earliest first and without repeats.
    fn instances(&self, r: &Reminder, after: i64, until: i64, max: usize) -> Vec<i64> {
        let zone = self.zone_of(r);
        let mut v: Vec<i64> = r
            .schedules
            .iter()
            .flat_map(|s| s.instances(&zone, after, until, max))
            .collect();
        v.sort_unstable();
        v.dedup();
        v.truncate(max);
        v
    }

    /// Fires every reminder whose time has come, opening an occurrence for
    /// each, and closes as missed the open occurrences whose expiry has come.
    /// A reminder whose time passed while the app was closed fires late, on
    /// the first tick after start; if its expiry has passed too it is missed
    /// at once and, as nobody should be alerted, not returned.
    pub fn tick(&mut self, now: i64) -> Result<Vec<Fired>> {
        let pending: Vec<(String, String, String, i64)> = self
            .states()
            .flat_map(|(list_id, s)| {
                s.pending_firings(now)
                    .into_iter()
                    .map(|r| {
                        (
                            list_id.to_string(),
                            r.id.clone(),
                            r.title.clone(),
                            r.fire_at,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut fired = Vec::new();
        for (list_id, reminder_id, title, scheduled_at) in pending {
            // The occurrence's identity is the reminder plus the scheduled
            // time, so firings on several devices merge into one.
            let occurrence_id = format!("{reminder_id}@{scheduled_at}");
            self.record_in(
                &list_id,
                now,
                Event::OccurrenceOpened {
                    occurrence_id: occurrence_id.clone(),
                    reminder_id: reminder_id.clone(),
                    scheduled_at,
                    fired_at: now,
                },
            )?;
            fired.push(Fired {
                occurrence_id,
                reminder_id,
                title,
            });
        }
        fired.extend(self.fire_schedules(now)?);
        fired.extend(self.fire_countdowns(now)?);
        let expired = self.expire_open(now)?;
        fired.retain(|f| !expired.contains(&f.occurrence_id));
        Ok(fired)
    }

    /// Closes as missed every open occurrence whose reminder's expiry delay,
    /// counted from the scheduled time, has passed. It is missed as of when
    /// the expiry came, not when this device noticed. Returns their ids.
    fn expire_open(&mut self, now: i64) -> Result<Vec<String>> {
        let due: Vec<(String, String, i64)> = self
            .states()
            .flat_map(|(list_id, s)| {
                s.occurrences
                    .values()
                    .filter(|o| o.is_open())
                    .filter_map(move |o| {
                        let at = s
                            .reminders
                            .get(&o.reminder_id)?
                            .expires_at(o.scheduled_at)?;
                        (at <= now).then(|| (list_id.to_string(), o.id.clone(), at))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut ids = Vec::new();
        for (list_id, id, at) in due {
            self.record_in(
                &list_id,
                now,
                Event::OccurrenceMissed {
                    occurrence_id: id.clone(),
                    missed_at: at,
                },
            )?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// When the next open occurrence goes overdue after `now`, so the window
    /// can move it from Due to Overdue, and the platform can escalate.
    pub fn next_overdue_at(&self, now: i64) -> Option<i64> {
        self.states()
            .flat_map(|(_, s)| {
                s.occurrences
                    .values()
                    .filter(|o| o.is_open())
                    .filter_map(|o| {
                        let at = s.reminders.get(&o.reminder_id)?.overdue_at(o.scheduled_at);
                        (at > now).then_some(at)
                    })
            })
            .min()
    }

    /// When a countdown reminder fires next: a set time after its latest
    /// occurrence closed (the closing's own time, so a completion recorded as
    /// 9:40 counts from 9:40), or after it was last done if it hasn't fired
    /// yet, or at once if it never was. `None` while an occurrence is open,
    /// which restarts the countdown when it closes (ADR 0001), and for a
    /// reminder that isn't a countdown.
    fn countdown_next(&self, state: &State, r: &Reminder) -> Option<i64> {
        let countdown = r.countdown.as_ref()?;
        let zone = self.zone_of(r);
        match state.latest_occurrence(&r.id) {
            Some(o) => {
                let closed = o.closing.as_ref()?.at;
                let next = countdown.next_after(&zone, closed)?;
                // Never the id of an occurrence it already had.
                Some(next.max(o.scheduled_at.saturating_add(1)))
            }
            None => match r.countdown_from {
                Some(done) => countdown.next_after(&zone, done),
                None => Some(r.created_at),
            },
        }
    }

    /// Fires the countdowns that have run out. Like a schedule it fires late
    /// on waking, with its scheduled time unchanged so what is overdue and
    /// what has expired count from it.
    fn fire_countdowns(&mut self, now: i64) -> Result<Vec<Fired>> {
        let mut due = Vec::new();
        for (list_id, state) in self.states() {
            for r in state.reminders.values().filter(|r| r.counts_down()) {
                if let Some(at) = self.countdown_next(state, r).filter(|at| *at <= now) {
                    due.push((list_id.to_string(), r.id.clone(), r.title.clone(), at));
                }
            }
        }
        let mut fired = Vec::new();
        for (list_id, reminder_id, title, at) in due {
            let occurrence_id = format!("{reminder_id}@{at}");
            self.record_in(
                &list_id,
                now,
                Event::OccurrenceOpened {
                    occurrence_id: occurrence_id.clone(),
                    reminder_id: reminder_id.clone(),
                    scheduled_at: at,
                    fired_at: now,
                },
            )?;
            fired.push(Fired {
                occurrence_id,
                reminder_id,
                title,
            });
        }
        Ok(fired)
    }

    /// Fires the schedules whose instants have come. A device that was off or
    /// asleep fires late, on waking: the latest instance fires, with its
    /// scheduled time unchanged so what is overdue and what has expired count
    /// from it. Earlier instances that passed meanwhile are recorded as
    /// missed, and so is any occurrence still open when an instance fires
    /// (ADR 0001): a reminder never has two open.
    fn fire_schedules(&mut self, now: i64) -> Result<Vec<Fired>> {
        struct Work {
            list_id: String,
            reminder: Reminder,
            instances: Vec<i64>,
        }
        let mut work = Vec::new();
        for (list_id, state) in self.states() {
            for r in state.reminders.values().filter(|r| r.has_schedules()) {
                let after = state
                    .last_scheduled(&r.id)
                    .unwrap_or(i64::MIN)
                    .max(r.active_from.saturating_sub(1));
                let mut instances = self.instances(r, after, now, MAX_INSTANCES);
                if instances.len() > MAX_LATE_INSTANCES {
                    instances.drain(..instances.len() - MAX_LATE_INSTANCES);
                }
                if !instances.is_empty() {
                    work.push(Work {
                        list_id: list_id.to_string(),
                        reminder: r.clone(),
                        instances,
                    });
                }
            }
        }
        let mut fired = Vec::new();
        for w in work {
            let reminder_id = &w.reminder.id;
            let last = *w.instances.last().expect("some instances");
            for at in w.instances {
                let occurrence_id = format!("{reminder_id}@{at}");
                let state = self.state_of(&w.list_id).expect("the list is held");
                if state.occurrences.contains_key(&occurrence_id) {
                    continue; // another device already fired this instance
                }
                let open = state
                    .occurrences
                    .values()
                    .find(|o| &o.reminder_id == reminder_id && o.is_open())
                    .map(|o| o.id.clone());
                if let Some(open) = open {
                    self.record_in(
                        &w.list_id,
                        now,
                        Event::OccurrenceMissed {
                            occurrence_id: open,
                            missed_at: at,
                        },
                    )?;
                }
                self.record_in(
                    &w.list_id,
                    now,
                    Event::OccurrenceOpened {
                        occurrence_id: occurrence_id.clone(),
                        reminder_id: reminder_id.clone(),
                        scheduled_at: at,
                        fired_at: now,
                    },
                )?;
                if at == last {
                    fired.push(Fired {
                        occurrence_id,
                        reminder_id: reminder_id.clone(),
                        title: w.reminder.title.clone(),
                    });
                }
            }
        }
        Ok(fired)
    }

    /// The list and id of an open occurrence, which may have merged into
    /// another's.
    fn open_id(&self, occurrence_id: &str) -> Result<(String, String)> {
        for (list_id, state) in self.states() {
            let id = state.resolve(occurrence_id).to_string();
            if let Some(o) = state.occurrences.get(&id) {
                return if o.is_open() {
                    Ok((list_id.to_string(), id))
                } else {
                    Err(Error::NotOpen(occurrence_id.to_string()))
                };
            }
        }
        Err(Error::NotOpen(occurrence_id.to_string()))
    }

    /// Completes an open occurrence, recording who and when. The one-off
    /// reminder is then finished.
    pub fn complete(&mut self, occurrence_id: &str, now: i64) -> Result<()> {
        self.complete_at(occurrence_id, now, now)
    }

    /// Completes an open occurrence as done at `completed_at`, which may be
    /// earlier than `now` ("I did it at 9:40"). A countdown restarts from it.
    pub fn complete_at(&mut self, occurrence_id: &str, completed_at: i64, now: i64) -> Result<()> {
        if completed_at > now {
            return Err(Error::InTheFuture);
        }
        let (list_id, id) = self.open_id(occurrence_id)?;
        self.record_in(
            &list_id,
            now,
            Event::OccurrenceCompleted {
                occurrence_id: id,
                completed_at,
            },
        )?;
        Ok(())
    }

    /// Closes a countdown reminder's coming occurrence before it fires: it
    /// is opened and closed at once, as of `closed_at`, so the countdown
    /// restarts from there and the pending firing is cancelled, on every
    /// device. Returns the occurrence's id.
    fn close_expected(
        &mut self,
        reminder_id: &str,
        closed_at: i64,
        now: i64,
        close: impl FnOnce(String) -> Event,
    ) -> Result<String> {
        if closed_at > now {
            return Err(Error::InTheFuture);
        }
        let list_id = self.list_of_reminder(reminder_id)?;
        let state = self.state_of(&list_id).expect("the list is held");
        let r = &state.reminders[reminder_id];
        if !r.counts_down() {
            return Err(Error::NotCountdown(reminder_id.to_string()));
        }
        // An open occurrence is completed or skipped as itself.
        let scheduled_at = self
            .countdown_next(state, r)
            .ok_or_else(|| Error::NotCountdown(reminder_id.to_string()))?;
        let occurrence_id = format!("{reminder_id}@{scheduled_at}");
        self.record_in(
            &list_id,
            now,
            Event::OccurrenceOpened {
                occurrence_id: occurrence_id.clone(),
                reminder_id: reminder_id.to_string(),
                scheduled_at,
                fired_at: now,
            },
        )?;
        self.record_in(&list_id, now, close(occurrence_id.clone()))?;
        Ok(occurrence_id)
    }

    /// Completes a countdown reminder ahead of time, as done at
    /// `completed_at`: it restarts from then and the pending firing is
    /// cancelled. If it has fired, complete the open occurrence instead.
    pub fn complete_expected(
        &mut self,
        reminder_id: &str,
        completed_at: i64,
        now: i64,
    ) -> Result<String> {
        self.close_expected(reminder_id, completed_at, now, |id| {
            Event::OccurrenceCompleted {
                occurrence_id: id,
                completed_at,
            }
        })
    }

    /// Skips a countdown reminder's coming occurrence: it restarts from now.
    pub fn skip_expected(
        &mut self,
        reminder_id: &str,
        note: Option<&str>,
        now: i64,
    ) -> Result<String> {
        let note = note
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        self.close_expected(reminder_id, now, now, |id| Event::OccurrenceSkipped {
            occurrence_id: id,
            skipped_at: now,
            note,
        })
    }

    /// Skips an open occurrence, with an optional note.
    pub fn skip(&mut self, occurrence_id: &str, note: Option<&str>, now: i64) -> Result<()> {
        let (list_id, id) = self.open_id(occurrence_id)?;
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        self.record_in(
            &list_id,
            now,
            Event::OccurrenceSkipped {
                occurrence_id: id,
                skipped_at: now,
                note: note.map(str::to_string),
            },
        )?;
        Ok(())
    }

    /// Closes an open occurrence as missed, because it expired.
    pub fn mark_missed(&mut self, occurrence_id: &str, now: i64) -> Result<()> {
        let (list_id, id) = self.open_id(occurrence_id)?;
        self.record_in(
            &list_id,
            now,
            Event::OccurrenceMissed {
                occurrence_id: id,
                missed_at: now,
            },
        )?;
        Ok(())
    }

    /// Quiets an open occurrence's alerts until `until`. The occurrence stays
    /// open and goes overdue on schedule, quietly. A snooze while one holds
    /// replaces it; there is no limit on repeated snoozes, and each is
    /// recorded ([`snooze_history`](Self::snooze_history)).
    pub fn snooze(&mut self, occurrence_id: &str, until: i64, now: i64) -> Result<()> {
        if until <= now {
            return Err(Error::SnoozeInThePast);
        }
        let (list_id, id) = self.open_id(occurrence_id)?;
        self.record_in(
            &list_id,
            now,
            Event::OccurrenceSnoozed {
                occurrence_id: id,
                until,
            },
        )?;
        Ok(())
    }

    /// One tap on Snooze: quiets an open occurrence's alerts for its
    /// priority's snooze length (see `PrioritySettings::snooze_length`).
    /// Returns when the snooze ends.
    pub fn snooze_for_interval(&mut self, occurrence_id: &str, now: i64) -> Result<i64> {
        let (list_id, id) = self.open_id(occurrence_id)?;
        let item = self
            .state_of(&list_id)
            .and_then(|s| s.due().into_iter().find(|d| d.occurrence_id == id))
            .ok_or_else(|| Error::NotOpen(occurrence_id.to_string()))?;
        let until = now
            + item
                .priority
                .settings()
                .snooze_length(item.overdue_at <= now);
        self.snooze(occurrence_id, until, now)?;
        Ok(until)
    }

    /// Snoozes an expected occurrence ahead of time until `until`: it still
    /// fires at `scheduled_at`, quietly, into the lists, and alerts when the
    /// snooze ends. Its overdue time and expiry still count from
    /// `scheduled_at`. Nothing is opened now: the snooze is an event of its
    /// own that the occurrence picks up when it opens, on any device. If it
    /// has fired already, this snoozes the open occurrence. Returns the
    /// occurrence's id.
    pub fn snooze_expected(
        &mut self,
        reminder_id: &str,
        scheduled_at: i64,
        until: i64,
        now: i64,
    ) -> Result<String> {
        if until <= now {
            return Err(Error::SnoozeInThePast);
        }
        let id = format!("{reminder_id}@{scheduled_at}");
        if self.open_id(&id).is_ok() {
            self.snooze(&id, until, now)?;
            return Ok(id);
        }
        let list_id = self.list_of_reminder(reminder_id)?;
        let listed = scheduled_at > now
            && self
                .expected(now, scheduled_at)
                .iter()
                .any(|e| e.reminder_id == reminder_id && e.scheduled_at == scheduled_at);
        if !listed {
            return Err(Error::NotExpected(id));
        }
        self.record_in(
            &list_id,
            now,
            Event::ExpectedOccurrenceSnoozed {
                reminder_id: reminder_id.to_string(),
                scheduled_at,
                until,
            },
        )?;
        Ok(id)
    }

    /// Every snooze of an occurrence, oldest first: what each was set to end
    /// on and how it actually ended (ran out, was replaced by another, or the
    /// occurrence closed first).
    pub fn snooze_history(&self, occurrence_id: &str, now: i64) -> Vec<SnoozeView> {
        self.states()
            .find(|(_, s)| {
                let id = s.resolve(occurrence_id);
                s.snoozes.iter().any(|z| z.occurrence_id == id)
            })
            .map(|(_, s)| s.snoozes_of(occurrence_id, now))
            .unwrap_or_default()
    }

    /// The snooze menu for an open occurrence: the priority's current
    /// interval, 1 hour and tomorrow morning, with its expiry if it has one.
    pub fn snooze_picker(&self, occurrence_id: &str, now: i64) -> Result<SnoozePicker> {
        let (list_id, id) = self.open_id(occurrence_id)?;
        let item = self
            .state_of(&list_id)
            .and_then(|s| s.due().into_iter().find(|d| d.occurrence_id == id))
            .ok_or_else(|| Error::NotOpen(occurrence_id.to_string()))?;
        let length = item
            .priority
            .settings()
            .snooze_length(item.overdue_at <= now);
        Ok(self.picker(length, now, item.expires_at, false, now))
    }

    /// The snooze menu for an expected occurrence ("snooze ahead"): the
    /// choices count from its scheduled time, since it fires then, quietly.
    pub fn snooze_picker_expected(
        &self,
        reminder_id: &str,
        scheduled_at: i64,
        now: i64,
    ) -> Result<SnoozePicker> {
        let list_id = self.list_of_reminder(reminder_id)?;
        let r = &self.state_of(&list_id).expect("the list is held").reminders[reminder_id];
        let length = r.priority.settings().snooze_length(false);
        Ok(self.picker(length, scheduled_at, r.expires_at(scheduled_at), true, now))
    }

    fn picker(
        &self,
        length: i64,
        from: i64,
        expires_at: Option<i64>,
        ahead: bool,
        now: i64,
    ) -> SnoozePicker {
        let zone = schedule::zone(&self.device_zone()).unwrap_or(TimeZone::UTC);
        let option = |kind, until: i64, seconds| SnoozeOption {
            kind,
            until,
            seconds,
            last_chance: expires_at.is_some_and(|e| last_chance_at(e, Some(now), until).is_some()),
        };
        let mut options = vec![option(SnoozeKind::Interval, from + length, Some(length))];
        if length != 3_600 {
            options.push(option(SnoozeKind::Hour, from + 3_600, Some(3_600)));
        }
        if let Some(t) = schedule::tomorrow_at(&zone, from, TOMORROW_MORNING_HOUR) {
            options.push(option(SnoozeKind::TomorrowMorning, t, None));
        }
        SnoozePicker {
            options,
            expires_at,
            last_chance_at: expires_at.map(|e| e - crate::state::LAST_CHANCE_LEAD),
            ahead,
            from,
        }
    }

    /// What the alarm window shows about an occurrence, or `None` once it is
    /// closed (or unknown).
    pub fn occurrence_view(&self, occurrence_id: &str) -> Option<OccurrenceView> {
        for (list_id, state) in self.states() {
            let id = state.resolve(occurrence_id).to_string();
            if let Some(d) = state.due().into_iter().find(|d| d.occurrence_id == id) {
                let list_name = if list_id == self.list_id {
                    None
                } else {
                    state.list_name.clone()
                };
                return Some(OccurrenceView {
                    occurrence_id: d.occurrence_id,
                    title: d.title,
                    list_name,
                    priority: d.priority,
                    scheduled_at: d.scheduled_at,
                    overdue_at: d.overdue_at,
                    snoozed_until: d.snoozed_until,
                    acknowledged_at: d.acknowledged_at,
                    expires_at: d.expires_at,
                });
            }
        }
        None
    }

    /// Silences an open occurrence's current alert on all of the user's devices.
    pub fn acknowledge(&mut self, occurrence_id: &str, now: i64) -> Result<()> {
        let (list_id, id) = self.open_id(occurrence_id)?;
        self.record_in(
            &list_id,
            now,
            Event::OccurrenceAcknowledged { occurrence_id: id },
        )?;
        Ok(())
    }

    /// Records in the history that this device alerted its user about an
    /// open occurrence in `style`: the first alert, or a change of style.
    /// Repeats aren't recorded.
    pub fn record_alert(&mut self, occurrence_id: &str, style: AlertStyle, now: i64) -> Result<()> {
        let (list_id, id) = self.open_id(occurrence_id)?;
        self.record_in(
            &list_id,
            now,
            Event::OccurrenceAlerted {
                occurrence_id: id,
                style,
            },
        )?;
        Ok(())
    }

    /// The style this device last recorded an alert in for the occurrence.
    pub fn last_alert_style(&self, occurrence_id: &str) -> Option<AlertStyle> {
        self.states()
            .find_map(|(_, s)| s.last_alert_style(occurrence_id, &self.device_id))
    }

    /// The clock for a change made now, later than any this device has seen.
    fn next_hlc(&self, list_id: &str, now: i64) -> Hlc {
        let state = self.state_of(list_id).unwrap_or(&self.state);
        Hlc::next(now, &self.device_id, state.latest_hlc())
    }

    /// Changes a reminder's settings. Each setting that differs is its own
    /// change, judged on its own against changes from other devices.
    pub fn edit_reminder(&mut self, reminder_id: &str, edit: EditReminder, now: i64) -> Result<()> {
        let list_id = self.list_of_reminder(reminder_id)?;
        let current = self.state_of(&list_id).unwrap().reminders[reminder_id].clone();
        let mut changes = Vec::new();
        if let Some(title) = edit.title {
            let title = title.trim().to_string();
            if title.is_empty() {
                return Err(Error::EmptyTitle);
            }
            if title != current.title {
                changes.push(Change::Title(title));
            }
        }
        if let Some(fire_at) = edit.fire_at.filter(|t| *t != current.fire_at) {
            changes.push(Change::FireAt(fire_at));
        }
        if let Some(note) = edit.note.filter(|n| *n != current.note) {
            changes.push(Change::Note(note));
        }
        if let Some(countdown) = edit
            .countdown
            .filter(|c| Some(c) != current.countdown.as_ref())
        {
            if !current.counts_down() {
                return Err(Error::NotCountdown(reminder_id.to_string()));
            }
            countdown.validate().map_err(Error::BadCountdown)?;
            changes.push(Change::Countdown(countdown));
        }
        if let Some(schedules) = edit
            .schedules
            .filter(|s| *s != current.schedules)
            .filter(|s| !(s.is_empty() && current.counts_down()))
        {
            if current.counts_down() {
                return Err(Error::BadSchedule(
                    "a countdown reminder has no schedules".into(),
                ));
            }
            let zone = edit.zone.as_ref().unwrap_or(&current.zone);
            check_schedules(&schedules, zone.as_deref())?;
            changes.push(Change::Schedules(schedules));
        }
        if let Some(zone) = edit.zone.filter(|z| *z != current.zone) {
            check_schedules(&current.schedules, zone.as_deref())?;
            changes.push(Change::Zone(zone));
        }
        if let Some(p) = edit.priority.filter(|p| *p != current.priority) {
            changes.push(Change::Priority(p));
        }
        for (new, now_value, make) in [
            (
                edit.overdue,
                current.overdue_override,
                Change::Overdue as fn(Option<i64>) -> Change,
            ),
            (edit.expiry, current.expiry_after, Change::Expiry),
        ] {
            if let Some(v) = new.filter(|v| *v != now_value) {
                if v.is_some_and(|d| d < 0) {
                    return Err(Error::BadDuration);
                }
                changes.push(make(v));
            }
        }
        for change in changes {
            let hlc = self.next_hlc(&list_id, now);
            self.record_in(
                &list_id,
                now,
                Event::ReminderEdited {
                    reminder_id: reminder_id.to_string(),
                    hlc,
                    change,
                },
            )?;
        }
        Ok(())
    }

    /// Brings back a value a setting once had, such as the one that lost to
    /// another device's change. It is a new change, so it wins now.
    pub fn restore_setting(
        &mut self,
        reminder_id: &str,
        setting: Setting,
        version_event_id: &str,
        now: i64,
    ) -> Result<()> {
        let list_id = self.list_of_reminder(reminder_id)?;
        let change = self
            .state_of(&list_id)
            .unwrap()
            .history(reminder_id, setting)
            .into_iter()
            .find(|v| v.event_id == version_event_id)
            .ok_or(Error::NoSuchVersion)?
            .change;
        let hlc = self.next_hlc(&list_id, now);
        self.record_in(
            &list_id,
            now,
            Event::ReminderEdited {
                reminder_id: reminder_id.to_string(),
                hlc,
                change,
            },
        )?;
        Ok(())
    }

    /// When the next unfired reminder is due or the next open occurrence
    /// expires, so the scheduler can sleep.
    pub fn next_fire_at(&self) -> Option<i64> {
        let one_off = self.states().filter_map(|(_, s)| s.next_fire_at()).min();
        let scheduled = self
            .states()
            .flat_map(|(_, s)| {
                s.reminders
                    .values()
                    .filter(|r| r.has_schedules())
                    .filter_map(|r| {
                        let after = s
                            .last_scheduled(&r.id)
                            .unwrap_or(i64::MIN)
                            .max(r.active_from.saturating_sub(1));
                        let until = after.max(0).saturating_add(5 * 366 * DAY);
                        self.instances(r, after, until, 1).first().copied()
                    })
            })
            .min();
        let counting = self
            .states()
            .flat_map(|(_, s)| {
                s.reminders
                    .values()
                    .filter_map(|r| self.countdown_next(s, r))
            })
            .min();
        let expiring = self
            .states()
            .flat_map(|(_, s)| {
                s.occurrences
                    .values()
                    .filter(|o| o.is_open())
                    .filter_map(|o| s.reminders.get(&o.reminder_id)?.expires_at(o.scheduled_at))
            })
            .min();
        one_off
            .into_iter()
            .chain(scheduled)
            .chain(counting)
            .chain(expiring)
            .min()
    }

    /// Occurrences predicted after `now` and up to `until`, earliest first:
    /// the one-offs yet to fire and the instances of every schedule. The
    /// views ask for the range they show.
    pub fn expected(&self, now: i64, until: i64) -> Vec<ExpectedItem> {
        let mut out = Vec::new();
        for (_, s) in self.states() {
            for r in s.reminders.values() {
                let times = if r.counts_down() {
                    // Only the next one: the one after depends on when
                    // this one is closed.
                    self.countdown_next(s, r)
                        .filter(|t| *t > now && *t <= until)
                        .into_iter()
                        .collect()
                } else if r.has_schedules() {
                    let after = s
                        .last_scheduled(&r.id)
                        .unwrap_or(i64::MIN)
                        .max(r.active_from.saturating_sub(1))
                        .max(now);
                    self.instances(r, after, until, 500)
                } else if r.fire_at > now
                    && r.fire_at <= until
                    && !s.occurrences.values().any(|o| o.reminder_id == r.id)
                {
                    vec![r.fire_at]
                } else {
                    Vec::new()
                };
                out.extend(times.into_iter().map(|t| ExpectedItem {
                    reminder_id: r.id.clone(),
                    title: r.title.clone(),
                    scheduled_at: t,
                    snoozed_until: s.expected_snooze(&r.id, t),
                    expires_at: r.expires_at(t),
                }));
            }
        }
        out.sort_by(|a, b| (a.scheduled_at, &a.reminder_id).cmp(&(b.scheduled_at, &b.reminder_id)));
        out
    }

    /// The Inbox's Later today and Earlier today, for the day `now` falls in
    /// on this device (in the device's time zone).
    pub fn inbox(&self, now: i64) -> Inbox {
        let zone = schedule::zone(&self.device_zone()).unwrap_or(TimeZone::UTC);
        let (start, end) = schedule::day_bounds(&zone, now);
        let mut earlier: Vec<EarlierItem> = self
            .states()
            .flat_map(|(_, s)| {
                s.occurrences.values().filter_map(move |o| {
                    let c = o.closing.as_ref().filter(|c| c.at >= start && c.at < end)?;
                    let r = s.reminders.get(&o.reminder_id)?;
                    Some(EarlierItem {
                        occurrence_id: o.id.clone(),
                        title: r.title.clone(),
                        scheduled_at: o.scheduled_at,
                        closed_at: c.at,
                        kind: c.kind,
                    })
                })
            })
            .collect();
        earlier
            .sort_by(|a, b| (b.closed_at, &b.occurrence_id).cmp(&(a.closed_at, &a.occurrence_id)));
        let (mut overdue, due): (Vec<DueItem>, Vec<DueItem>) = self
            .states()
            .flat_map(|(_, s)| s.due())
            .partition(|d| d.overdue_at <= now);
        overdue.sort_by(|a, b| {
            (
                std::cmp::Reverse(a.priority),
                a.overdue_at,
                &a.occurrence_id,
            )
                .cmp(&(
                    std::cmp::Reverse(b.priority),
                    b.overdue_at,
                    &b.occurrence_id,
                ))
        });
        let mut due = due;
        due.sort_by(|a, b| (a.fired_at, &a.occurrence_id).cmp(&(b.fired_at, &b.occurrence_id)));
        Inbox {
            overdue,
            due,
            later_today: self.expected(now, end - 1),
            earlier_today: earlier,
        }
    }

    /// Records that this device is called `name`, in the personal list.
    pub fn name_device(&mut self, name: &str, now: i64) -> Result<()> {
        self.record(
            now,
            Event::DeviceNamed {
                name: name.trim().to_string(),
            },
        )?;
        Ok(())
    }

    /// Records that this device has just signed in to the account as `name`,
    /// so the user's other devices can tell them.
    pub fn announce_sign_in(&mut self, name: &str, now: i64) -> Result<()> {
        self.record(
            now,
            Event::DeviceSignedIn {
                name: name.trim().to_string(),
            },
        )?;
        Ok(())
    }

    /// Records that this device removed `device_id` from the account, so the
    /// user's other devices stop offering to remove it.
    pub fn record_device_removed(&mut self, device_id: &str, now: i64) -> Result<()> {
        self.record(
            now,
            Event::DeviceRemoved {
                device_id: device_id.to_string(),
            },
        )?;
        Ok(())
    }

    /// What a device of this account calls itself, if it has said.
    pub fn device_name(&self, device_id: &str) -> Option<&str> {
        self.state.device_names.get(device_id).map(String::as_str)
    }

    /// Sign-ins by other devices since this one signed in, which the user
    /// hasn't dismissed. A device doesn't announce what happened before it
    /// came, nor its own sign-in.
    fn sign_in_notices(&self) -> Vec<SignInNotice> {
        let sign_ins = &self.state.sign_ins;
        let own = sign_ins.iter().position(|s| s.device_id == self.device_id);
        let after = own.map_or(0, |i| i + 1);
        sign_ins[after..]
            .iter()
            .filter(|s| s.device_id != self.device_id)
            .filter(|s| !self.state.removed_devices.contains(&s.device_id))
            .filter(|s| {
                !matches!(
                    self.store.meta(&format!("dismissed:{}", s.event_id)),
                    Ok(Some(_))
                )
            })
            .map(|s| SignInNotice {
                id: s.event_id.clone(),
                device_id: s.device_id.clone(),
                device_name: s.name.clone(),
                at: s.at,
            })
            .collect()
    }

    /// The newest server notice this device has dealt with, if it has asked
    /// the server before.
    pub fn server_notice_cursor(&self) -> Result<Option<i64>> {
        Ok(self
            .store
            .meta("server_notice_cursor")?
            .and_then(|v| v.parse().ok()))
    }

    pub fn set_server_notice_cursor(&self, id: i64) -> Result<()> {
        self.store.set_meta("server_notice_cursor", &id.to_string())
    }

    /// Keep a notice the server sent. Kinds this app doesn't know are left
    /// out, as an update will bring them. Returns whether it is new.
    pub fn receive_server_notice(&self, id: i64, kind: &str, count: u32, at: i64) -> Result<bool> {
        if kind != FAILED_SIGN_INS {
            return Ok(false);
        }
        let key = format!("{SERVER_NOTICE}{id}");
        if self.store.meta(&key)?.is_some() {
            return Ok(false);
        }
        self.store.set_meta(&key, &format!("{kind}:{count}:{at}"))?;
        Ok(true)
    }

    fn security_notices(&self) -> Vec<SecurityNotice> {
        let Ok(rows) = self.store.meta_prefix(SERVER_NOTICE) else {
            return Vec::new();
        };
        let mut notices: Vec<(i64, SecurityNotice)> = rows
            .into_iter()
            .filter_map(|(key, value)| {
                let n: i64 = key.strip_prefix(SERVER_NOTICE)?.parse().ok()?;
                let mut parts = value.splitn(3, ':');
                let (kind, count, at) = (parts.next()?, parts.next()?, parts.next()?);
                if kind != FAILED_SIGN_INS {
                    return None;
                }
                let id = format!("server-notice-{n}");
                if matches!(self.store.meta(&format!("dismissed:{id}")), Ok(Some(_))) {
                    return None;
                }
                let count: u32 = count.parse().ok()?;
                Some((
                    n,
                    SecurityNotice {
                        text: format!("{count} failed sign-ins to your account"),
                        id,
                        count,
                        at: at.parse().ok()?,
                    },
                ))
            })
            .collect();
        notices.sort_by_key(|(n, _)| *n);
        notices.into_iter().map(|(_, n)| n).collect()
    }

    /// The user has seen a sign-in notice.
    pub fn dismiss_notice(&self, id: &str) -> Result<()> {
        self.store.set_meta(&format!("dismissed:{id}"), "1")
    }

    /// Closings by this user's devices that lost to a completion, which the
    /// user hasn't dismissed.
    fn reconciliation_notices(&self) -> Vec<ReconciliationNotice> {
        self.states()
            .flat_map(|(_, state)| state.reconciliations.iter().map(move |r| (state, r)))
            .filter(|(_, r)| r.by == self.user_id)
            .filter(|(_, r)| {
                !matches!(self.store.meta(&format!("dismissed:{}", r.id)), Ok(Some(_)))
            })
            .map(|(state, r)| {
                let title = state
                    .occurrences
                    .get(&r.occurrence_id)
                    .and_then(|o| state.reminders.get(&o.reminder_id))
                    .map_or("a reminder", |rem| rem.title.as_str());
                let device_name = self.device_name(&r.device_id).map(str::to_string);
                let device = device_name.as_deref().unwrap_or("another device");
                let did = match r.lost {
                    ClosingKind::Skipped => "skipped",
                    ClosingKind::Missed => "missed",
                    ClosingKind::Completed => "completed",
                };
                ReconciliationNotice {
                    id: r.id.clone(),
                    occurrence_id: r.occurrence_id.clone(),
                    device_id: r.device_id.clone(),
                    text: format!(
                        "Your {device} {did} \u{201c}{title}\u{201d}. It counts as completed."
                    ),
                    device_name,
                }
            })
            .collect()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            reconciliations: self.reconciliation_notices(),
            sign_in_notices: self.sign_in_notices(),
            security_notices: self.security_notices(),
            due: {
                let mut due: Vec<DueItem> = self.states().flat_map(|(_, s)| s.due()).collect();
                due.sort_by_key(|d| (d.fired_at, d.occurrence_id.clone()));
                due
            },
            upcoming: {
                let mut up: Vec<UpcomingItem> =
                    self.states().flat_map(|(_, s)| s.upcoming()).collect();
                up.sort_by_key(|u| (u.fire_at, u.reminder_id.clone()));
                up
            },
            countdowns: {
                let mut v: Vec<CountdownItem> = self
                    .states()
                    .flat_map(|(_, s)| {
                        s.reminders.values().filter_map(move |r| {
                            Some(CountdownItem {
                                reminder_id: r.id.clone(),
                                title: r.title.clone(),
                                countdown: r.countdown.clone()?,
                                next_at: self.countdown_next(s, r),
                            })
                        })
                    })
                    .collect();
                v.sort_by_key(|c| (c.next_at.unwrap_or(i64::MAX), c.reminder_id.clone()));
                v
            },
            notices: self.device_notices(),
            update_notice: self.holding_newer.then(|| UPDATE_NOTICE.to_string()),
        }
    }
}

/// Checks a reminder's schedules and zone can be used.
fn check_schedules(schedules: &[Schedule], zone: Option<&str>) -> Result<()> {
    if let Some(z) = zone {
        if schedule::zone(z).is_none() {
            return Err(Error::BadZone(z.to_string()));
        }
    }
    for s in schedules {
        s.validate().map_err(Error::BadSchedule)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_000_000;

    fn core() -> Core {
        Core::open_in_memory().unwrap()
    }

    /// Deliver `from`'s unsent events to `to` as the server would number them.
    fn deliver(from: &mut Core, to: &mut [&mut Core], device: &str, first_seq: i64) {
        for (i, o) in from.unsent().unwrap().into_iter().enumerate() {
            let seq = first_seq + i as i64;
            for t in to.iter_mut() {
                t.receive(&o.list_id, seq, &o.event_id, device, o.format, &o.payload)
                    .unwrap();
            }
            from.mark_sent(&o.event_id, seq).unwrap();
        }
    }

    #[test]
    fn a_device_hears_of_later_sign_ins_but_not_its_own_or_earlier_ones() {
        // Device 1 is the first; 2 signs in; 3 signs in later.
        let mut one = core();
        one.join("u1", "1").unwrap();
        let list = one.personal_list_id().to_string();
        one.name_device("Desktop", T0).unwrap();
        let mut two = core();
        two.join("u1", "2").unwrap();
        two.use_personal_list(&list).unwrap();
        let mut three = core();
        three.join("u1", "3").unwrap();
        three.use_personal_list(&list).unwrap();

        // The server numbers them 1, 2, 3 and everyone downloads them.
        deliver(&mut one, &mut [&mut two, &mut three], "1", 1);
        two.announce_sign_in("Laptop", T0 + 10).unwrap();
        deliver(&mut two, &mut [&mut one, &mut three], "2", 2);
        three.announce_sign_in("Tablet", T0 + 20).unwrap();
        deliver(&mut three, &mut [&mut one, &mut two], "3", 3);

        // Names come from the user's own settings events.
        assert_eq!(one.device_name("1"), Some("Desktop"));
        assert_eq!(one.device_name("2"), Some("Laptop"));
        assert_eq!(three.device_name("3"), Some("Tablet"));

        let names = |c: &Core| -> Vec<String> {
            c.snapshot()
                .sign_in_notices
                .into_iter()
                .map(|n| n.device_name)
                .collect()
        };
        assert_eq!(names(&one), vec!["Laptop", "Tablet"]);
        assert_eq!(names(&two), vec!["Tablet"]);
        assert!(names(&three).is_empty());

        let notice = one.snapshot().sign_in_notices[0].clone();
        assert_eq!((notice.device_id.as_str(), notice.at), ("2", T0 + 10));
        one.dismiss_notice(&notice.id).unwrap();
        assert_eq!(names(&one), vec!["Tablet"]);
    }

    #[test]
    fn a_server_notice_shows_until_dismissed_and_is_kept_once() {
        let c = core();
        assert_eq!(c.server_notice_cursor().unwrap(), None);
        assert!(c
            .receive_server_notice(3, "failed_sign_ins", 5, T0)
            .unwrap());
        assert!(!c
            .receive_server_notice(3, "failed_sign_ins", 5, T0)
            .unwrap());
        assert!(c
            .receive_server_notice(12, "failed_sign_ins", 10, T0 + 9)
            .unwrap());
        // A kind from a newer server is left out.
        assert!(!c.receive_server_notice(13, "something_new", 1, T0).unwrap());
        c.set_server_notice_cursor(13).unwrap();
        assert_eq!(c.server_notice_cursor().unwrap(), Some(13));

        let shown = c.snapshot().security_notices;
        let texts: Vec<_> = shown.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "5 failed sign-ins to your account",
                "10 failed sign-ins to your account"
            ]
        );
        assert_eq!((shown[0].count, shown[0].at), (5, T0));
        c.dismiss_notice(&shown[0].id).unwrap();
        let left = c.snapshot().security_notices;
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].count, 10);
    }

    #[test]
    fn removing_a_device_takes_its_sign_in_notice_off_every_device() {
        let mut one = core();
        one.join("u1", "1").unwrap();
        let list = one.personal_list_id().to_string();
        let mut two = core();
        two.join("u1", "2").unwrap();
        two.use_personal_list(&list).unwrap();
        let mut three = core();
        three.join("u1", "3").unwrap();
        three.use_personal_list(&list).unwrap();
        two.announce_sign_in("Laptop", T0 + 10).unwrap();
        deliver(&mut two, &mut [&mut one, &mut three], "2", 1);
        three.announce_sign_in("Phone", T0 + 20).unwrap();
        deliver(&mut three, &mut [&mut one, &mut two], "3", 2);
        assert_eq!(one.snapshot().sign_in_notices.len(), 2);
        assert_eq!(two.snapshot().sign_in_notices.len(), 1);

        // The first device removes the phone.
        one.record_device_removed("3", T0 + 30).unwrap();
        deliver(&mut one, &mut [&mut two, &mut three], "1", 3);
        for c in [&one, &two] {
            let left: Vec<_> = c
                .snapshot()
                .sign_in_notices
                .into_iter()
                .map(|n| n.device_name)
                .collect();
            assert!(!left.contains(&"Phone".to_string()), "{left:?}");
        }
        assert_eq!(one.snapshot().sign_in_notices.len(), 1);
        // Its name is still known, for the history.
        assert_eq!(one.device_name("3"), Some("Phone"));
    }

    #[test]
    fn reminder_does_not_fire_before_its_time() {
        let mut c = core();
        c.create_reminder("Call the plumber", T0 + 60, T0).unwrap();
        assert!(c.tick(T0 + 59).unwrap().is_empty());
        assert!(c.snapshot().due.is_empty());
        assert_eq!(c.snapshot().upcoming.len(), 1);
        assert_eq!(c.next_fire_at(), Some(T0 + 60));
    }

    #[test]
    fn reminder_fires_at_its_time_and_opens_an_occurrence() {
        let mut c = core();
        c.create_reminder("Call the plumber", T0 + 60, T0).unwrap();
        let fired = c.tick(T0 + 60).unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].title, "Call the plumber");
        let snap = c.snapshot();
        assert_eq!(snap.due.len(), 1);
        assert_eq!(snap.due[0].title, "Call the plumber");
        assert_eq!(snap.due[0].scheduled_at, T0 + 60);
        assert!(snap.upcoming.is_empty());
        assert_eq!(c.next_fire_at(), None);
    }

    #[test]
    fn firing_happens_once() {
        let mut c = core();
        c.create_reminder("Once", T0, T0).unwrap();
        assert_eq!(c.tick(T0).unwrap().len(), 1);
        assert!(c.tick(T0 + 1).unwrap().is_empty());
        assert_eq!(c.snapshot().due.len(), 1);
    }

    #[test]
    fn completing_records_who_and_when_and_finishes_the_one_off() {
        let mut c = core();
        let rid = c.create_reminder("Bins", T0, T0).unwrap();
        let fired = c.tick(T0).unwrap();
        c.complete(&fired[0].occurrence_id, T0 + 30).unwrap();

        assert!(c.snapshot().due.is_empty());
        let o = &c.state().occurrences[&fired[0].occurrence_id];
        assert_eq!(o.completed(), Some((c.user_id().to_string(), T0 + 30)));
        assert!(c.state().is_finished(&rid));
        // Finished: it never fires again.
        assert!(c.tick(T0 + 1000).unwrap().is_empty());
        assert_eq!(c.next_fire_at(), None);
    }

    #[test]
    fn completing_twice_or_unknown_is_an_error() {
        let mut c = core();
        c.create_reminder("Bins", T0, T0).unwrap();
        let fired = c.tick(T0).unwrap();
        c.complete(&fired[0].occurrence_id, T0).unwrap();
        assert!(matches!(
            c.complete(&fired[0].occurrence_id, T0 + 1),
            Err(Error::NotOpen(_))
        ));
        assert!(matches!(c.complete("nope", T0), Err(Error::NotOpen(_))));
    }

    #[test]
    fn empty_title_is_rejected() {
        let mut c = core();
        assert!(matches!(
            c.create_reminder("   ", T0, T0),
            Err(Error::EmptyTitle)
        ));
    }

    #[test]
    fn same_time_reminders_each_fire() {
        let mut c = core();
        c.create_reminder("A", T0, T0).unwrap();
        c.create_reminder("B", T0, T0).unwrap();
        assert_eq!(c.tick(T0).unwrap().len(), 2);
        assert_eq!(c.snapshot().due.len(), 2);
    }

    #[test]
    fn restart_rebuilds_the_same_state_and_fires_what_was_missed() {
        let dir = std::env::temp_dir().join(format!("hab-core-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hab.db");

        let (open_id, done_id);
        {
            let mut c = Core::open(&path).unwrap();
            c.create_reminder("Open one", T0 + 10, T0).unwrap();
            c.create_reminder("Done one", T0 + 10, T0).unwrap();
            c.create_reminder("Later", T0 + 5000, T0).unwrap();
            c.create_reminder("While closed", T0 + 100, T0).unwrap();
            let fired = c.tick(T0 + 10).unwrap();
            assert_eq!(fired.len(), 2);
            let done = fired.iter().find(|f| f.title == "Done one").unwrap();
            let open = fired.iter().find(|f| f.title == "Open one").unwrap();
            c.complete(&done.occurrence_id, T0 + 20).unwrap();
            done_id = done.occurrence_id.clone();
            open_id = open.occurrence_id.clone();
        }

        // Restart after "While closed" was due: the same state, rebuilt from
        // the stream, and the missed reminder fires on the first tick.
        let mut c = Core::open(&path).unwrap();
        assert_eq!(c.snapshot().due.len(), 1);
        assert_eq!(c.snapshot().due[0].occurrence_id, open_id);
        assert!(c.state().occurrences[&done_id].completed().is_some());
        assert_eq!(c.snapshot().upcoming.len(), 2);

        let fired = c.tick(T0 + 200).unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].title, "While closed");
        assert_eq!(c.snapshot().due.len(), 2);
        assert_eq!(c.snapshot().upcoming[0].title, "Later");

        // Restarting again changes nothing and fires nothing.
        drop(c);
        let mut c = Core::open(&path).unwrap();
        assert!(c.tick(T0 + 200).unwrap().is_empty());
        assert_eq!(c.snapshot().due.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_change_is_an_event_numbered_in_the_stream() {
        let mut c = core();
        c.create_reminder("X", T0, T0).unwrap();
        let fired = c.tick(T0).unwrap();
        c.complete(&fired[0].occurrence_id, T0 + 1).unwrap();
        let events = c.store.stream(&c.list_id).unwrap();
        let seqs: Vec<Option<i64>> = events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![Some(1), Some(2), Some(3)]);
        assert!(matches!(events[0].event, Event::ReminderCreated { .. }));
        assert!(matches!(events[1].event, Event::OccurrenceOpened { .. }));
        assert!(matches!(events[2].event, Event::OccurrenceCompleted { .. }));
        assert!(events.iter().all(|e| e.author == c.user_id));
    }

    #[test]
    fn duplicate_firings_from_another_device_merge() {
        // The occurrence id is the reminder plus the scheduled time, so a
        // second device's identical firing changes nothing.
        let mut c = core();
        let rid = c.create_reminder("Shared", T0, T0).unwrap();
        c.tick(T0).unwrap();
        let dup = StoredEvent {
            list_id: c.list_id.clone(),
            seq: Some(99),
            event_id: "other".into(),
            device_id: "other-device".into(),
            author: c.user_id.clone(),
            recorded_at: T0 + 5,
            event: Event::OccurrenceOpened {
                occurrence_id: format!("{rid}@{T0}"),
                reminder_id: rid,
                scheduled_at: T0,
                fired_at: T0 + 5,
            },
        };
        c.state.apply(&dup);
        assert_eq!(c.state().occurrences.len(), 1);
        assert_eq!(c.snapshot().due[0].fired_at, T0);
    }

    #[test]
    fn after_joining_new_changes_wait_for_the_server_to_number_them() {
        let mut c = core();
        let rid = c.create_reminder("Before joining", T0 + 50, T0).unwrap();
        assert!(!c.snapshot().upcoming[0].not_sent);
        c.join("u1", "7").unwrap();
        // The standalone history is now unsent, and is uploaded in order.
        assert!(c.snapshot().upcoming[0].not_sent);
        c.tick(T0 + 50).unwrap();
        let out = c.unsent().unwrap();
        assert_eq!(out.len(), 2);
        let first: Payload = serde_json::from_slice(&out[0].payload).unwrap();
        assert_eq!((first.author.as_str(), first.recorded_at), ("u1", T0));
        assert_eq!(first.event["reminder_id"], rid);
        // The server's numbers replace the local ones, wherever they fall.
        c.mark_sent(&out[0].event_id, 5).unwrap();
        assert!(c.snapshot().due[0].not_sent);
        c.mark_sent(&out[1].event_id, 6).unwrap();
        assert!(!c.snapshot().due[0].not_sent);
        assert!(c.unsent().unwrap().is_empty());
    }

    #[test]
    fn events_from_a_newer_format_are_kept_and_applied_after_an_update() {
        let mut c = core();
        c.join("u1", "7").unwrap();
        let payload = |event: serde_json::Value| {
            serde_json::to_vec(&Payload {
                author: "u1".into(),
                recorded_at: T0,
                event,
            })
            .unwrap()
        };
        let list = c.personal_list_id().to_string();
        let future = payload(serde_json::json!({"type": "hologram", "n": 1}));
        assert!(c
            .receive(&list, 1, "f1", "8", FORMAT_VERSION + 1, &future)
            .unwrap());
        assert_eq!(c.snapshot().update_notice.as_deref(), Some(UPDATE_NOTICE));
        assert!(c.snapshot().upcoming.is_empty());
        // An event in a known format that doesn't read is refused, not kept.
        let junk = payload(serde_json::json!({"type": "hologram"}));
        assert!(c
            .receive(&list, 2, "bad", "8", FORMAT_VERSION, &junk)
            .is_err());
        // Receiving the same event again changes nothing.
        assert!(!c
            .receive(&list, 1, "f1", "8", FORMAT_VERSION + 1, &future)
            .unwrap());
        assert_eq!(c.held_events().unwrap(), vec!["f1".to_string()]);
    }

    #[test]
    fn a_new_device_takes_the_accounts_list_only_while_its_own_is_empty() {
        let mut c = core();
        c.use_personal_list("account-list").unwrap();
        assert_eq!(c.personal_list_id(), "account-list");
        c.create_reminder("X", T0, T0).unwrap();
        assert!(c.use_personal_list("other").is_err());
        assert_eq!(c.personal_list_id(), "account-list");
    }

    #[test]
    fn linking_a_standalone_device_keeps_its_list_apart_and_named() {
        let mut c = core();
        let standalone = c.personal_list_id().to_string();
        let id = c.create_reminder("Bins", T0, T0).unwrap();
        let fired = c.tick(T0).unwrap();
        c.complete(&fired[0].occurrence_id, T0 + 1).unwrap();
        c.set_personal_setting("quiet_hours", "22:00-07:00")
            .unwrap();
        c.set_device_setting("loudest_style", "alarm").unwrap();
        c.join("u1", "7").unwrap();

        c.link_account("account-list", "Laptop", T0 + 2).unwrap();
        assert_eq!(c.personal_list_id(), "account-list");
        let lists = c.lists();
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[1].id, standalone);
        assert_eq!(lists[1].name.as_deref(), Some("Laptop"));
        // Nothing of it is in the account's list; its history is untouched.
        assert!(c.state().reminders.is_empty());
        let s = c.state_of(&standalone).unwrap();
        assert!(s.reminders.contains_key(&id));
        assert!(!s.occurrences[&fired[0].occurrence_id].is_open());
        // Every event of it is waiting to be sent, to its own list.
        let unsent = c.unsent().unwrap();
        assert_eq!(unsent.len(), 4);
        assert!(unsent.iter().all(|o| o.list_id == standalone));
        // The personal setting gave way and the user is told; the device's stayed.
        assert_eq!(c.personal_setting("quiet_hours").unwrap(), None);
        assert_eq!(
            c.device_setting("loudest_style").unwrap().as_deref(),
            Some("alarm")
        );
        assert_eq!(c.snapshot().notices.len(), 1);
        c.dismiss_notice(SETTINGS_GAVE_WAY).unwrap();
        assert!(c.snapshot().notices.is_empty());
        // Running it again changes nothing.
        c.link_account("account-list", "Laptop", T0 + 3).unwrap();
        assert_eq!(c.unsent().unwrap().len(), 4);
        // It survives a restart of the app's database.
        assert_eq!(c.lists().len(), 2);
    }

    #[test]
    fn linking_a_device_with_nothing_made_standalone_adds_no_list_and_no_notice() {
        let mut c = core();
        c.set_device_setting("loudest_style", "alarm").unwrap();
        c.join("u1", "7").unwrap();
        c.link_account("account-list", "Laptop", T0).unwrap();
        assert_eq!(c.personal_list_id(), "account-list");
        assert_eq!(c.lists().len(), 1);
        assert!(c.snapshot().notices.is_empty());
        assert!(c.unsent().unwrap().is_empty());
    }

    #[test]
    fn a_link_interrupted_after_naming_the_list_finishes_without_naming_it_twice() {
        let mut c = core();
        c.create_reminder("Bins", T0, T0).unwrap();
        c.join("u1", "7").unwrap();
        // The first attempt got as far as naming the list.
        let standalone = c.personal_list_id().to_string();
        c.record(
            T0,
            Event::ListNamed {
                name: "Laptop".into(),
            },
        )
        .unwrap();
        c.link_account("account-list", "Laptop", T0 + 1).unwrap();
        let names = c
            .unsent()
            .unwrap()
            .into_iter()
            .filter(|o| {
                o.list_id == standalone && o.payload.windows(10).any(|w| w == b"list_named")
            })
            .count();
        assert_eq!(names, 1);
    }

    #[test]
    fn reminders_in_an_imported_list_fire_and_are_acted_on_where_they_live() {
        let mut c = core();
        let standalone = c.personal_list_id().to_string();
        c.create_reminder("Later", T0 + 100, T0).unwrap();
        c.join("u1", "7").unwrap();
        c.link_account("account-list", "Laptop", T0).unwrap();
        assert_eq!(c.next_fire_at(), Some(T0 + 100));
        let fired = c.tick(T0 + 100).unwrap();
        assert_eq!(fired.len(), 1);
        assert!(c
            .state_of(&standalone)
            .unwrap()
            .occurrences
            .contains_key(&fired[0].occurrence_id));
        assert!(c.state().occurrences.is_empty());
        c.snooze(&fired[0].occurrence_id, T0 + 500, T0 + 101)
            .unwrap();
        c.skip(&fired[0].occurrence_id, None, T0 + 102).unwrap();
        assert!(c.snapshot().due.is_empty());
    }
}
