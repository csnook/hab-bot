use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jiff::tz::TimeZone;
use serde::Serialize;
use uuid::Uuid;

use crate::condition::{self, Condition};
use crate::countdown::Countdown;
use crate::delay::{Delay, DelaySpec};
use crate::device::{DeviceLimits, DeviceQuiet, LoudestAlert};
use crate::event::{
    Change, Correction, Event, Outgoing, Payload, Setting, StoredEvent, UndoOutcome,
    FORMAT_VERSION, UPDATE_NOTICE,
};
use crate::hlc::Hlc;
use crate::pause::{Pause, PauseCause};
use crate::place::Place;
use crate::priority::{AlertStyle, Priority};
use crate::quiet::{QuietHours, Scope, SnoozeAll, Source};
use crate::schedule::{self, Parts, Schedule};
use crate::state::{
    last_chance_at, ClosingKind, DueItem, HistoryWhat, Occurrence, Reminder, SnoozeView, State,
    UpcomingItem,
};
use crate::store::Store;
use crate::sun::SunTrigger;
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
    /// The reminder's note.
    pub note: String,
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

/// How long a snooze-all made from a menu lasts (the dialog also takes any
/// time of the user's choosing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnoozeAllChoice {
    Minutes(i64),
    /// 08:00 the next day.
    TomorrowMorning,
}

/// A snooze-all or stretch of quiet hours holding now, for the toolbar chip
/// and the sidebar footer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnoozeAllView {
    /// What ends a snooze-all early; quiet hours have none.
    pub id: Option<String>,
    pub source: Source,
    pub scope: Scope,
    /// The list's name for a list scope; `None` for the personal list's too.
    pub list_name: Option<String>,
    pub include_maximum: bool,
    pub from: i64,
    pub until: i64,
}

/// The hour tomorrow morning means, in the device's time zone.
const TOMORROW_MORNING_HOUR: i8 = 8;

/// A reminder list this device holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListInfo {
    pub id: String,
    /// What the list is called. The personal list is not named.
    pub name: Option<String>,
    /// The list's colour, `#rrggbb`, if it has been given one.
    pub colour: Option<String>,
    /// The account's personal list, which also holds its settings.
    pub personal: bool,
    /// How many reminders it holds, not counting deleted ones.
    pub reminders: usize,
    /// The list's pause, if it was ever paused and not resumed. It sets its
    /// reminders aside only for as long as it covers the time
    /// ([`Pause::covers`]): one that has run out is still shown here.
    pub pause: Option<Pause>,
}

/// A reminder that was deleted with its history kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeletedReminder {
    pub reminder_id: String,
    pub list_id: String,
    pub title: String,
    /// When it was deleted, in Unix seconds.
    pub deleted_at: i64,
    /// The user who deleted it.
    pub deleted_by: String,
}

/// What the window leaves out, by the sidebar's list and priority checkboxes.
/// What is hidden is listed rather than what is shown, so a list or priority
/// made later shows. It only decides what the window lists: every alert
/// still comes, as the [`crate::Alerter`] never looks at it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Filters {
    pub hidden_lists: Vec<String>,
    pub hidden_priorities: Vec<Priority>,
}

const FILTERS: &str = "filters";
/// The loudest alert style this device uses, as its snake_case name.
const LOUDEST_STYLE: &str = "loudest_style";
/// "1" when the loudest style caps Maximum too.
const LOUDEST_CAPS_MAXIMUM: &str = "loudest_caps_maximum";
/// "Quiet this device until…", as JSON ([`DeviceQuiet`]).
const QUIET_DEVICE: &str = "quiet_device";

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
    /// The list the reminder is in.
    pub list_id: String,
    pub priority: Priority,
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
    /// Overrides the priority's overdue time with a delay from the scheduled
    /// time (a duration, or the next time a schedule matches), or with
    /// `Some(None)` goes back to following it.
    pub overdue: Option<Option<Delay>>,
    /// All the delays after which an open occurrence is missed, whichever
    /// comes first, replacing the reminder's. Empty is no such expiry.
    pub expiry: Option<Vec<Delay>>,
    /// A new countdown, for a countdown reminder.
    pub countdown: Option<Countdown>,
    /// All the sun-event triggers, replacing the reminder's.
    pub suns: Option<Vec<SunTrigger>>,
    /// All the time-based conditions, replacing the reminder's.
    pub conditions: Option<Vec<Condition>>,
}

/// What fires a reminder, as the editor's When shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerView {
    /// Fires once, at a time.
    OneOff {
        fire_at: i64,
    },
    /// Fires on schedules.
    Schedules {
        schedules: Vec<ScheduleView>,
        /// Sun events it also fires at.
        suns: Vec<SunTrigger>,
    },
    Countdown {
        countdown: Countdown,
    },
}

/// One schedule trigger: the editor's pattern if it is one, and its rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScheduleView {
    /// `None` for a rule written by hand that the editor can't show.
    pub parts: Option<Parts>,
    pub start: String,
    pub rule: String,
}

/// A reminder as the editor shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReminderView {
    pub reminder_id: String,
    pub list_id: String,
    /// The list's name; `None` for the personal list.
    pub list_name: Option<String>,
    /// The list's colour, if it has one.
    pub list_colour: Option<String>,
    pub title: String,
    pub note: String,
    pub priority: Priority,
    pub trigger: TriggerView,
    /// The zone it is pinned to; `None` is floating.
    pub zone: Option<String>,
    /// How long after its scheduled time an occurrence goes overdue by
    /// default: its priority's, shown greyed until overridden.
    pub default_overdue_seconds: i64,
    /// The overdue override; `None` follows the priority.
    pub overdue: Option<DelaySpec>,
    /// The expiries added, whichever comes first. Firing again always
    /// expires an occurrence too.
    pub expiries: Vec<DelaySpec>,
    /// The reminder's own pause, if it was paused and not resumed. It counts
    /// only while it covers the time ([`Pause::covers`]).
    pub pause: Option<Pause>,
    /// Its list's pause, which also sets it aside.
    pub list_pause: Option<Pause>,
    /// The time-based conditions it fires only within.
    pub conditions: Vec<Condition>,
    /// It has a sun event or a daylight or darkness condition and the user
    /// hasn't set a home location, so those can't work yet.
    pub needs_home: bool,
}

/// An occurrence predicted to come due: it becomes an occurrence only if the
/// reminder fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExpectedItem {
    pub reminder_id: String,
    /// The list the reminder is in.
    pub list_id: String,
    pub priority: Priority,
    pub title: String,
    pub note: String,
    pub scheduled_at: i64,
    /// Snoozed ahead of time until then: it fires at its time, quietly.
    pub snoozed_until: Option<i64>,
    /// When it would be missed for want of action, if it has such an expiry.
    pub expires_at: Option<i64>,
    /// "Complete early" and "Skip ahead" are offered: it is the reminder's
    /// next expected occurrence and nothing of the reminder is open.
    pub can_close_early: bool,
}

/// A closed occurrence for the Inbox's Earlier today section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EarlierItem {
    pub occurrence_id: String,
    /// The list the reminder is in.
    pub list_id: String,
    pub priority: Priority,
    pub title: String,
    pub scheduled_at: i64,
    pub closed_at: i64,
    pub kind: ClosingKind,
    /// A completion or skip can be undone; a miss is corrected instead.
    pub can_undo: bool,
    /// How it was closed was changed afterwards: the history has the original.
    pub corrected: bool,
    /// It was skipped because it fell in a pause.
    pub paused: Option<PauseCause>,
}

/// How a closed occurrence counts, for the history and reliability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Completed while due.
    DoneOnTime,
    /// Completed while overdue, including a miss corrected to completed.
    DoneLate,
    Skipped,
    Missed,
}

/// One closing, correction or undo in a closed occurrence's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClosedEntry {
    pub event_id: String,
    pub what: HistoryWhat,
    /// The time the user said it was done or skipped.
    pub at: Option<i64>,
    pub note: Option<String>,
    /// Who; empty if the app closed it (a miss).
    pub by: String,
    /// When it was tapped.
    pub tapped_at: i64,
    /// When the server received it; `None` until it has.
    pub received_at: Option<i64>,
    /// A later correction or undo took its place: the original, kept.
    pub superseded: bool,
    /// A skip the pause made: the history attributes it to the pause.
    pub paused: Option<PauseCause>,
}

/// A closed occurrence as the details panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClosedView {
    pub occurrence_id: String,
    pub reminder_id: String,
    pub list_id: String,
    pub title: String,
    pub priority: Priority,
    pub scheduled_at: i64,
    pub kind: ClosingKind,
    pub outcome: Outcome,
    /// The time the user said.
    pub at: i64,
    pub note: Option<String>,
    pub can_undo: bool,
    pub corrected: bool,
    /// It was skipped because it fell in a pause.
    pub paused: Option<PauseCause>,
    /// Every closing, correction and undo, oldest first.
    pub history: Vec<ClosedEntry>,
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
    /// Open occurrences of reminders that are paused now. They stay open and
    /// can be acted on, but they are not in Overdue or Due, never alert and
    /// don't count in the tray's badge: pausing is for being left alone.
    pub paused: Vec<PausedOpen>,
}

/// An open occurrence of a paused reminder, and what pauses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PausedOpen {
    pub item: DueItem,
    pub pause: PauseCause,
}

/// A reminder that is paused now, for the Board's Paused column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PausedReminder {
    pub reminder_id: String,
    pub list_id: String,
    pub title: String,
    pub priority: Priority,
    /// What pauses it: its own pause or its list's, and until when.
    pub pause: PauseCause,
    /// When that pause began.
    pub from: i64,
    /// It has an open occurrence, which stays where it is until it closes
    /// (the Board keeps such a card in Overdue or Due).
    pub has_open: bool,
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
        let mut all: BTreeMap<String, State> = BTreeMap::new();
        let mut holding_newer = false;
        let mut ids = self.store.list_ids()?;
        if !ids.contains(&self.list_id) {
            ids.push(self.list_id.clone());
        }
        for id in ids {
            holding_newer |= !self.store.held(&id)?.is_empty();
            let mut state = State::default();
            for e in self.store.stream(&id)? {
                state.apply(&e);
            }
            all.insert(id, state);
        }
        settle(&mut all);
        self.state = all.remove(&self.list_id).unwrap_or_default();
        self.others = all;
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
    /// others by name. A list the device has no events of yet is not held, and
    /// a deleted one that is still empty is gone.
    pub fn lists(&self) -> Vec<ListInfo> {
        let info = |id: &str, s: &State, personal: bool| ListInfo {
            id: id.to_string(),
            name: if personal { None } else { s.list_name.clone() },
            colour: s.list_colour.clone(),
            personal,
            reminders: s.reminders.len(),
            pause: s.list_pause,
        };
        let mut v = vec![info(&self.list_id, &self.state, true)];
        let mut others: Vec<ListInfo> = self
            .others
            .iter()
            .filter(|(_, s)| !s.list_gone())
            .map(|(id, s)| info(id, s, false))
            .collect();
        others.sort_by_cached_key(|l| {
            (
                l.name.clone().unwrap_or_default().to_lowercase(),
                l.id.clone(),
            )
        });
        v.extend(others);
        v
    }

    /// The list exists here, and is not gone.
    fn check_list(&self, list_id: &str) -> Result<()> {
        match self.state_of(list_id) {
            Some(s) if list_id == self.list_id || !s.list_gone() => Ok(()),
            _ => Err(Error::NoList(list_id.to_string())),
        }
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
        if inserted {
            self.forget_purged(event_id, &p.event)?;
        }
        self.rebuild()?;
        Ok(inserted)
    }

    /// An event arrived. If it purges a reminder, what this device stored of
    /// that reminder goes; and an event about a reminder already purged is
    /// dropped as it comes, unless it is the purge itself.
    fn forget_purged(&mut self, event_id: &str, event: &serde_json::Value) -> Result<()> {
        if event.get("type").and_then(|t| t.as_str()) == Some("reminder_purged") {
            if let Some(id) = event.get("reminder_id").and_then(|i| i.as_str()) {
                self.store.redact_reminder(id)?;
            }
            return Ok(());
        }
        let text = event.to_string();
        let purged: Vec<String> = self
            .states()
            .flat_map(|(_, s)| s.purged.iter().cloned())
            .collect();
        if purged.iter().any(|id| text.contains(id.as_str())) {
            self.store.delete_event(event_id)?;
        }
        Ok(())
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

    /// A reminder as the editor shows it.
    pub fn reminder_view(&self, reminder_id: &str) -> Result<ReminderView> {
        let list_id = self.list_of_reminder(reminder_id)?;
        let state = self.state_of(&list_id).unwrap();
        let r = &state.reminders[reminder_id];
        let trigger = if let Some(c) = &r.countdown {
            TriggerView::Countdown {
                countdown: c.clone(),
            }
        } else if r.has_schedules() {
            TriggerView::Schedules {
                schedules: r
                    .schedules
                    .iter()
                    .map(|s| ScheduleView {
                        parts: s.parts(),
                        start: s.start.clone(),
                        rule: s.rule.clone(),
                    })
                    .collect(),
                suns: r.suns.clone(),
            }
        } else {
            TriggerView::OneOff { fire_at: r.fire_at }
        };
        Ok(ReminderView {
            reminder_id: r.id.clone(),
            list_name: if list_id == self.list_id {
                None
            } else {
                state.list_name.clone()
            },
            list_colour: state.list_colour.clone(),
            list_id,
            title: r.title.clone(),
            note: r.note.clone(),
            priority: r.priority,
            trigger,
            zone: r.zone.clone(),
            default_overdue_seconds: r.default_overdue_after(),
            overdue: r.overdue_override.as_ref().map(Delay::spec),
            expiries: r.expiries.iter().map(Delay::spec).collect(),
            pause: r.pause,
            list_pause: state.list_pause,
            conditions: r.conditions.clone(),
            needs_home: r.needs_home() && self.home().is_none(),
        })
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
        let list_id = self.list_id.clone();
        self.create_reminder_in(&list_id, title, fire_at, now)
    }

    /// Creates a one-off reminder in the list `list_id`.
    pub fn create_reminder_in(
        &mut self,
        list_id: &str,
        title: &str,
        fire_at: i64,
        now: i64,
    ) -> Result<String> {
        self.check_list(list_id)?;
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::EmptyTitle);
        }
        let reminder_id = Uuid::new_v4().to_string();
        self.record_in(
            list_id,
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
        let list_id = self.list_id.clone();
        self.create_recurring_reminder_in(&list_id, title, schedules, zone, now)
    }

    /// [`Self::create_recurring_reminder`] in the list `list_id`.
    pub fn create_recurring_reminder_in(
        &mut self,
        list_id: &str,
        title: &str,
        schedules: Vec<Schedule>,
        zone: Option<&str>,
        now: i64,
    ) -> Result<String> {
        self.create_repeating_reminder_in(
            list_id,
            title,
            schedules,
            Vec::new(),
            Vec::new(),
            zone,
            now,
        )
    }

    /// Creates a reminder in the list `list_id` that fires on `schedules` and
    /// at the sun events `suns` (at least one of them), and only where
    /// `conditions` hold: an instant outside them passes, with no occurrence
    /// and no waiting. Sun events and daylight conditions use the user's
    /// home location ([`Core::set_home`]); with none set a sun event never
    /// fires and a daylight condition counts as met.
    #[allow(clippy::too_many_arguments)]
    pub fn create_repeating_reminder_in(
        &mut self,
        list_id: &str,
        title: &str,
        schedules: Vec<Schedule>,
        suns: Vec<SunTrigger>,
        conditions: Vec<Condition>,
        zone: Option<&str>,
        now: i64,
    ) -> Result<String> {
        self.check_list(list_id)?;
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::EmptyTitle);
        }
        check_schedules(&schedules, zone)?;
        check_suns(&suns)?;
        check_conditions(&conditions)?;
        if schedules.is_empty() && suns.is_empty() {
            return Err(Error::BadSchedule(
                "a repeating reminder needs a schedule or a sun event".into(),
            ));
        }
        let reminder_id = Uuid::new_v4().to_string();
        self.record_in(
            list_id,
            now,
            Event::RecurringReminderCreated {
                reminder_id: reminder_id.clone(),
                title: title.to_string(),
                schedules,
                zone: zone.map(str::to_string),
                suns,
                conditions,
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
        let list_id = self.list_id.clone();
        self.create_countdown_reminder_in(&list_id, title, countdown, zone, last_done, now)
    }

    /// [`Self::create_countdown_reminder`] in the list `list_id`.
    pub fn create_countdown_reminder_in(
        &mut self,
        list_id: &str,
        title: &str,
        countdown: Countdown,
        zone: Option<&str>,
        last_done: Option<i64>,
        now: i64,
    ) -> Result<String> {
        self.check_list(list_id)?;
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
        self.record_in(
            list_id,
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

    /// The device's own time zone.
    fn device_tz(&self) -> TimeZone {
        schedule::zone(&self.device_zone()).unwrap_or(TimeZone::UTC)
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
    ///
    /// These are the reminder's scheduled instants that pass its time-based
    /// conditions, evaluated here and now as a pure function of the instant,
    /// the reminder's zone and the user's home location, so every device
    /// holding the same data agrees. An instant outside the conditions is
    /// simply absent: it fires nothing, makes nothing wait, and isn't
    /// predicted. Sun events come from the home location, and only after it
    /// was set.
    fn instances(&self, r: &Reminder, after: i64, until: i64, max: usize) -> Vec<i64> {
        let zone = self.zone_of(r);
        let home = self.home();
        let since = self.home_since().unwrap_or(i64::MAX);
        // What the conditions drop is dropped after the schedules have given
        // their instants, so they must give more than `max`.
        let cap = if r.conditions.is_empty() && r.suns.is_empty() {
            max
        } else {
            max.max(MAX_INSTANCES)
        };
        let window = |lo: i64, hi: i64| -> Vec<i64> {
            let mut v: Vec<i64> = r
                .schedules
                .iter()
                .flat_map(|s| s.instances(&zone, lo, hi, cap))
                .collect();
            if let Some(h) = &home {
                // Sun events don't fire for what passed before the home
                // location was set.
                let from = lo.max(since.saturating_sub(1));
                for sun in &r.suns {
                    v.extend(sun.instants(h.latitude, h.longitude, from, hi));
                }
            }
            v.retain(|t| condition::all_hold(&r.conditions, *t, &zone, home.as_ref()));
            v.sort_unstable();
            v.dedup();
            v
        };
        if r.conditions.is_empty() && r.suns.is_empty() {
            let mut v = window(after, until);
            v.truncate(max);
            return v;
        }
        // Conditions and sun events thin out or generate the instants, so
        // they are found a stretch at a time, growing, until there are
        // enough or the range is done.
        let mut out: Vec<i64> = Vec::new();
        let mut lo = after.max(until.saturating_sub(60 * 366 * DAY));
        let mut step = 31 * DAY;
        while lo < until && out.len() < max {
            let hi = lo.saturating_add(step).min(until);
            out.extend(window(lo, hi));
            lo = hi;
            step = (step * 2).min(366 * DAY);
        }
        out.truncate(max);
        out
    }

    /// The user's home location, if they have set one: their Home place,
    /// which syncs to all their devices (ADR 0012).
    pub fn home(&self) -> Option<Place> {
        self.state.home.clone()
    }

    /// When the home location in use was set, in Unix seconds.
    fn home_since(&self) -> Option<i64> {
        self.state.home.as_ref().and(self.state.home_since)
    }

    /// Sets the user's home location, the centre of their Home place with
    /// the default radius, in degrees (north and east are positive). It is a
    /// personal setting: it syncs to all their devices. Sun events fire for
    /// instants after this.
    pub fn set_home(&mut self, latitude: f64, longitude: f64, now: i64) -> Result<()> {
        let place = Place::home(latitude, longitude).map_err(Error::BadHome)?;
        if self.state.home.as_ref() == Some(&place) {
            return Ok(());
        }
        let list_id = self.list_id.clone();
        let hlc = self.next_hlc(&list_id, now);
        self.record_in(
            &list_id,
            now,
            Event::HomeSet {
                hlc,
                place: Some(place),
            },
        )?;
        Ok(())
    }

    /// Clears the home location: sun events stop firing and daylight and
    /// darkness conditions count as met.
    pub fn clear_home(&mut self, now: i64) -> Result<()> {
        if self.state.home.is_none() {
            return Ok(());
        }
        let list_id = self.list_id.clone();
        let hlc = self.next_hlc(&list_id, now);
        self.record_in(&list_id, now, Event::HomeSet { hlc, place: None })?;
        Ok(())
    }

    /// Fires every reminder whose time has come, opening an occurrence for
    /// each, and closes as missed the open occurrences whose expiry has come.
    /// A reminder whose time passed while the app was closed fires late, on
    /// the first tick after start; if its expiry has passed too it is missed
    /// at once and, as nobody should be alerted, not returned.
    ///
    /// An instance that falls in a pause doesn't fire: it is recorded as
    /// skipped because of the pause (ADR 0011).
    pub fn tick(&mut self, now: i64) -> Result<Vec<Fired>> {
        let mut fired = self.fire_due(now, false)?;
        let expired = self.expire_open(now)?;
        fired.retain(|f| !expired.contains(&f.occurrence_id));
        Ok(fired)
    }

    /// Records as skipped the instances that have come due in a pause, and
    /// nothing else. Run before a pause changes, so that what fell in the
    /// pause as it stood is skipped whatever the change, and a device waking
    /// after a resume doesn't fire what the pause had covered.
    fn skip_paused(&mut self, now: i64) -> Result<()> {
        self.fire_due(now, true).map(|_| ())
    }

    /// Opens (or, in a pause, skips) every instance whose time has come. With
    /// `paused_only` an instance outside a pause is left alone.
    fn fire_due(&mut self, now: i64, paused_only: bool) -> Result<Vec<Fired>> {
        let pending: Vec<(String, String, String, i64, Option<PauseCause>)> = self
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
                            s.pause_at(r, r.fire_at),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut fired = Vec::new();
        for (list_id, reminder_id, title, scheduled_at, paused) in pending {
            if let Some(cause) = paused {
                self.record_pause_skip(
                    &list_id,
                    &reminder_id,
                    scheduled_at,
                    scheduled_at,
                    cause,
                    now,
                )?;
                continue;
            }
            if paused_only {
                continue;
            }
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
        fired.extend(self.fire_schedules(now, paused_only)?);
        fired.extend(self.fire_countdowns(now, paused_only)?);
        Ok(fired)
    }

    /// Records that an occurrence, open or yet to open, was skipped because
    /// it fell in a pause, as of `skipped_at`.
    fn record_pause_skip(
        &mut self,
        list_id: &str,
        reminder_id: &str,
        scheduled_at: i64,
        skipped_at: i64,
        cause: PauseCause,
        now: i64,
    ) -> Result<()> {
        self.record_in(
            list_id,
            now,
            Event::OccurrenceSkippedForPause {
                occurrence_id: format!("{reminder_id}@{scheduled_at}"),
                reminder_id: reminder_id.to_string(),
                scheduled_at,
                skipped_at,
                until: cause.until,
                list: cause.list,
            },
        )?;
        Ok(())
    }

    /// Closes as missed every open occurrence whose reminder's expiry delay,
    /// counted from the scheduled time, has passed. It is missed as of when
    /// the expiry came, not when this device noticed. Returns their ids.
    fn expire_open(&mut self, now: i64) -> Result<Vec<String>> {
        let device = &self.device_tz();
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
                            .expires_at(o.scheduled_at, device)?;
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
        let device = &self.device_tz();
        self.states()
            .flat_map(|(_, s)| {
                s.occurrences
                    .values()
                    .filter(|o| o.is_open())
                    .filter_map(|o| {
                        let at = s
                            .reminders
                            .get(&o.reminder_id)?
                            .overdue_at(o.scheduled_at, device);
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
            // Closed ahead of its time and undone: it comes back as expected.
            Some(o) if o.unfired => Some(o.scheduled_at),
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
    /// what has expired count from it. One that runs out in a pause is
    /// skipped as of when it ran out, which restarts it from there (ADR 0011).
    fn fire_countdowns(&mut self, now: i64, paused_only: bool) -> Result<Vec<Fired>> {
        let mut fired = Vec::new();
        // Each skip restarts the countdown, which may run out again in the
        // same pause.
        for _ in 0..MAX_LATE_INSTANCES {
            let mut due = Vec::new();
            for (list_id, state) in self.states() {
                for r in state.reminders.values().filter(|r| r.counts_down()) {
                    if let Some(at) = self.countdown_next(state, r).filter(|at| *at <= now) {
                        due.push((
                            list_id.to_string(),
                            r.id.clone(),
                            r.title.clone(),
                            at,
                            state.pause_at(r, at),
                        ));
                    }
                }
            }
            let mut progressed = false;
            for (list_id, reminder_id, title, at, paused) in due {
                if let Some(cause) = paused {
                    self.record_pause_skip(&list_id, &reminder_id, at, at, cause, now)?;
                    progressed = true;
                    continue;
                }
                if paused_only {
                    continue;
                }
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
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        Ok(fired)
    }

    /// Fires the schedules whose instants have come. A device that was off or
    /// asleep fires late, on waking: the latest instance fires, with its
    /// scheduled time unchanged so what is overdue and what has expired count
    /// from it. Earlier instances that passed meanwhile are recorded as
    /// missed, and so is any occurrence still open when an instance fires
    /// (ADR 0001): a reminder never has two open. An instance in a pause is
    /// skipped instead, as of its own time (ADR 0011).
    fn fire_schedules(&mut self, now: i64, paused_only: bool) -> Result<Vec<Fired>> {
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
                if state.is_fired(&occurrence_id) {
                    continue; // another device already fired this instance
                }
                if let Some(cause) = state.pause_at(&w.reminder, at) {
                    self.record_pause_skip(&w.list_id, reminder_id, at, at, cause, now)?;
                    continue;
                }
                if paused_only {
                    // What comes after is for the next tick to fire.
                    break;
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
                // A deleted reminder's occurrences are history only.
                return if o.is_open() && state.reminders.contains_key(&o.reminder_id) {
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

    /// When the reminder's next occurrence is expected, if it has one that
    /// can be closed ahead of time: a countdown's, a schedule's next instance
    /// (once nothing is open) or a one-off's, yet to fire.
    fn next_expected(&self, state: &State, r: &Reminder, now: i64) -> Result<i64> {
        let open = state
            .occurrences
            .values()
            .any(|o| o.reminder_id == r.id && o.is_open());
        if open {
            return Err(Error::StillOpen(r.id.clone()));
        }
        let next = if r.counts_down() {
            self.countdown_next(state, r)
        } else if r.has_schedules() {
            let after = state
                .last_scheduled(&r.id)
                .unwrap_or(i64::MIN)
                .max(r.active_from.saturating_sub(1))
                .max(now);
            let until = after.max(0).saturating_add(5 * 366 * DAY);
            self.instances(r, after, until, 1).first().copied()
        } else if !state.has_fired(&r.id) {
            Some(r.fire_at)
        } else {
            None
        };
        next.ok_or_else(|| Error::NotExpected(r.id.clone()))
    }

    /// Closes a reminder's coming occurrence before it fires: it is opened
    /// and closed at once, as of `closed_at`, so it never fires, on any
    /// device, and a countdown restarts from there. Returns the occurrence's
    /// id.
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
        // An open occurrence is completed or skipped as itself.
        let scheduled_at = self.next_expected(state, r, now)?;
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

    /// Completes a reminder's next expected occurrence ahead of time, as done
    /// at `completed_at`: it never fires, and a countdown restarts from then.
    /// A reminder with none expected, such as one that fires when someone
    /// arrives somewhere, doesn't offer it. If it has fired, complete the
    /// open occurrence instead.
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

    /// Skips a reminder's next expected occurrence ahead of time: it never
    /// fires, and a countdown restarts from now.
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

    /// Whether the reminder has an expected occurrence that can be completed
    /// or skipped ahead of time, which is when "Complete early" and "Skip
    /// ahead" are offered.
    pub fn offers_early(&self, reminder_id: &str, now: i64) -> bool {
        let Ok(list_id) = self.list_of_reminder(reminder_id) else {
            return false;
        };
        let state = self.state_of(&list_id).expect("the list is held");
        let r = &state.reminders[reminder_id];
        self.next_expected(state, r, now).is_ok()
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

    /// The list, id and state of a closed occurrence of a live reminder.
    fn closed_id(&self, occurrence_id: &str) -> Result<(String, String)> {
        for (list_id, state) in self.states() {
            let id = state.resolve(occurrence_id).to_string();
            if let Some(o) = state.occurrences.get(&id) {
                return if o.closing.is_some() && state.reminders.contains_key(&o.reminder_id) {
                    Ok((list_id.to_string(), id))
                } else {
                    Err(Error::NotClosed(occurrence_id.to_string()))
                };
            }
        }
        Err(Error::NotClosed(occurrence_id.to_string()))
    }

    /// What undoing a closing would leave, worked out here and recorded in
    /// the undo so every device reaches the same answer: reopened if it would
    /// still be open (not expired, and no newer occurrence fired); back to
    /// being expected if it was closed ahead of its time and that time hasn't
    /// come; otherwise missed.
    fn undo_outcome(&self, state: &State, o: &Occurrence, now: i64) -> UndoOutcome {
        if o.is_early() && now < o.scheduled_at {
            return UndoOutcome::Expected;
        }
        let newer = state
            .occurrences
            .values()
            .filter(|n| {
                n.reminder_id == o.reminder_id
                    && !n.unfired
                    && (n.scheduled_at, &n.id) > (o.scheduled_at, &o.id)
            })
            .map(|n| n.scheduled_at)
            .max();
        if let Some(at) = newer {
            return UndoOutcome::Missed {
                at: at.max(o.scheduled_at),
            };
        }
        let expiry = state
            .reminders
            .get(&o.reminder_id)
            .and_then(|r| r.expires_at(o.scheduled_at, &self.device_tz()))
            .filter(|at| *at <= now);
        match expiry {
            Some(at) => UndoOutcome::Missed { at },
            None => UndoOutcome::Reopened,
        }
    }

    /// Undoes a completion or skip. The occurrence opens again if it would
    /// still be open; closed ahead of its time it is expected again, and a
    /// countdown goes back to what it was; otherwise it becomes missed, as of
    /// when it expired or the newer occurrence fired, and a countdown counts
    /// from that. Returns what it left. A miss isn't undone but corrected.
    ///
    /// It takes the place of the closings this device has seen. One another
    /// device made meanwhile stands, so that a deliberate undo is never read
    /// as the devices disagreeing.
    pub fn undo(&mut self, occurrence_id: &str, now: i64) -> Result<UndoOutcome> {
        let (list_id, id) = self.closed_id(occurrence_id)?;
        let state = self.state_of(&list_id).expect("the list is held");
        let o = &state.occurrences[&id];
        if o.closing
            .as_ref()
            .is_some_and(|c| c.kind == ClosingKind::Missed)
        {
            return Err(Error::CantUndoMiss);
        }
        let outcome = self.undo_outcome(state, o, now);
        let replaces = state.standing_closings(&id);
        self.record_in(
            &list_id,
            now,
            Event::OccurrenceUndone {
                occurrence_id: id,
                replaces,
                outcome: outcome.clone(),
            },
        )?;
        Ok(outcome)
    }

    /// Changes how a closed occurrence, a missed one too, was closed: to
    /// completed or skipped, as of `at`. The history keeps the original and
    /// the correction, and a miss corrected to completed counts as done
    /// late. A countdown restarts from `at`.
    pub fn correct(
        &mut self,
        occurrence_id: &str,
        kind: Correction,
        at: i64,
        note: Option<&str>,
        now: i64,
    ) -> Result<()> {
        if at > now {
            return Err(Error::InTheFuture);
        }
        let (list_id, id) = self.closed_id(occurrence_id)?;
        let replaces = self
            .state_of(&list_id)
            .expect("the list is held")
            .standing_closings(&id);
        let note = note
            .map(str::trim)
            .filter(|n| !n.is_empty() && kind == Correction::Skipped)
            .map(str::to_string);
        self.record_in(
            &list_id,
            now,
            Event::OccurrenceCorrected {
                occurrence_id: id,
                replaces,
                kind,
                at,
                note,
            },
        )?;
        Ok(())
    }

    /// A closed occurrence for the details panel, with its history.
    pub fn closed_occurrence(&self, occurrence_id: &str) -> Option<ClosedView> {
        let (list_id, id) = self.closed_id(occurrence_id).ok()?;
        let state = self.state_of(&list_id)?;
        let o = &state.occurrences[&id];
        let r = &state.reminders[&o.reminder_id];
        let c = o.closing.as_ref()?;
        let history: Vec<ClosedEntry> = state
            .history_of(&id)
            .into_iter()
            .map(|h| ClosedEntry {
                received_at: self.store.received_at(&h.event_id).ok().flatten(),
                event_id: h.event_id,
                what: h.what,
                at: h.at,
                note: h.note,
                by: h.by,
                tapped_at: h.recorded_at,
                superseded: h.superseded,
                paused: h.paused,
            })
            .collect();
        let corrected = !c.replaces.is_empty();
        // Did a miss come before it, through however many corrections?
        let mut from_miss = false;
        let mut seen = c.replaces.clone();
        while let Some(e) = seen.pop() {
            if let Some(prior) = o.records.iter().find(|p| p.event_id == e) {
                from_miss |= prior.kind == ClosingKind::Missed;
                seen.extend(prior.replaces.iter().cloned());
            }
        }
        let outcome = match c.kind {
            ClosingKind::Completed
                if from_miss || c.at >= r.overdue_at(o.scheduled_at, &self.device_tz()) =>
            {
                Outcome::DoneLate
            }
            ClosingKind::Completed => Outcome::DoneOnTime,
            ClosingKind::Skipped => Outcome::Skipped,
            ClosingKind::Missed => Outcome::Missed,
        };
        Some(ClosedView {
            occurrence_id: id,
            reminder_id: r.id.clone(),
            list_id,
            title: r.title.clone(),
            priority: r.priority,
            scheduled_at: o.scheduled_at,
            kind: c.kind,
            outcome,
            at: c.at,
            note: c.note.clone(),
            can_undo: c.kind != ClosingKind::Missed,
            corrected,
            paused: c.paused,
            history,
        })
    }

    /// Notes given when skipping, most recent first, to offer again.
    pub fn recent_skip_notes(&self, limit: usize) -> Vec<String> {
        let mut all: Vec<(i64, String)> = self
            .states()
            .flat_map(|(_, s)| s.recent_skip_notes(limit))
            .collect();
        all.sort_by(|a, b| b.cmp(a));
        let mut seen = BTreeSet::new();
        all.retain(|(_, n)| seen.insert(n.clone()));
        all.into_iter().take(limit).map(|(_, n)| n).collect()
    }

    /// The server numbered an event and says when it received it, which the
    /// history shows beside the time it was said and the time it was tapped.
    pub fn set_received_at(&self, event_id: &str, received_at: i64) -> Result<()> {
        self.store.set_received_at(event_id, received_at)
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
            .and_then(|s| {
                s.due(&self.device_tz())
                    .into_iter()
                    .find(|d| d.occurrence_id == id)
            })
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
        let zone = self.device_tz();
        let Some((_, s)) = self
            .states()
            .find(|(_, s)| s.occurrences.contains_key(s.resolve(occurrence_id)))
        else {
            return Vec::new();
        };
        let id = s.resolve(occurrence_id).to_string();
        let mut out = s.snoozes_of(&id, now);
        if let Some((o, r)) = s
            .occurrences
            .get(&id)
            .and_then(|o| Some((o, s.reminders.get(&o.reminder_id)?)))
        {
            // What a snooze-all or quiet hours held of it, which the
            // personal settings say (ADR 0013).
            let closed_at = o.closing.as_ref().map(|c| c.at);
            let mut held = self.state.holds_over(
                &zone,
                &r.list_id,
                r.priority,
                o.scheduled_at,
                closed_at,
                now,
            );
            for v in &mut held {
                v.occurrence_id = id.clone();
            }
            out.extend(held);
            out.sort_by_key(|v| (v.set_at, v.until));
        }
        out
    }

    /// The snooze menu for an open occurrence: the priority's current
    /// interval, 1 hour and tomorrow morning, with its expiry if it has one.
    pub fn snooze_picker(&self, occurrence_id: &str, now: i64) -> Result<SnoozePicker> {
        let (list_id, id) = self.open_id(occurrence_id)?;
        let item = self
            .state_of(&list_id)
            .and_then(|s| {
                s.due(&self.device_tz())
                    .into_iter()
                    .find(|d| d.occurrence_id == id)
            })
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
        Ok(self.picker(
            length,
            scheduled_at,
            r.expires_at(scheduled_at, &self.device_tz()),
            true,
            now,
        ))
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
            if let Some(d) = state
                .due(&self.device_tz())
                .into_iter()
                .find(|d| d.occurrence_id == id)
            {
                let list_name = if list_id == self.list_id {
                    None
                } else {
                    state.list_name.clone()
                };
                return Some(OccurrenceView {
                    occurrence_id: d.occurrence_id,
                    title: d.title,
                    note: d.note,
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
        let schedules_after = edit.schedules.clone().unwrap_or(current.schedules.clone());
        let suns_after = edit.suns.clone().unwrap_or(current.suns.clone());
        let repeats_on_time = !schedules_after.is_empty() || !suns_after.is_empty();
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
        if let Some(suns) = edit.suns.filter(|s| *s != current.suns) {
            if current.counts_down() {
                return Err(Error::BadSunEvent(
                    "a countdown reminder has no sun events".into(),
                ));
            }
            check_suns(&suns)?;
            if suns.is_empty() && schedules_after.is_empty() && current.has_schedules() {
                return Err(Error::BadSunEvent(
                    "a repeating reminder needs a schedule or a sun event".into(),
                ));
            }
            changes.push(Change::SunEvents(suns));
        }
        if let Some(conditions) = edit.conditions.filter(|c| *c != current.conditions) {
            if !conditions.is_empty() && (current.counts_down() || !repeats_on_time) {
                return Err(Error::BadCondition(
                    "time-based conditions apply to reminders that repeat on a schedule or at a sun event".into(),
                ));
            }
            check_conditions(&conditions)?;
            changes.push(Change::Conditions(conditions));
        }
        if let Some(p) = edit.priority.filter(|p| *p != current.priority) {
            changes.push(Change::Priority(p));
        }
        if let Some(v) = edit.overdue.filter(|v| *v != current.overdue_override) {
            if let Some(d) = &v {
                check_delay(d)?;
            }
            changes.push(Change::Overdue(v));
        }
        if let Some(v) = edit.expiry.filter(|v| *v != current.expiries) {
            for d in &v {
                check_delay(d)?;
            }
            changes.push(Change::Expiry(v));
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
    ///
    /// An instance in a pause counts too: nothing alerts, but it is skipped
    /// when its time comes, so that the history says so and a countdown
    /// restarts, and for that the device wakes (ADR 0011).
    pub fn next_fire_at(&self) -> Option<i64> {
        let device = &self.device_tz();
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
                    .filter_map(|o| {
                        s.reminders
                            .get(&o.reminder_id)?
                            .expires_at(o.scheduled_at, device)
                    })
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
        let device = &self.device_tz();
        let mut out = Vec::new();
        for (_, s) in self.states() {
            for r in s.reminders.values() {
                let times: Vec<i64> = if r.counts_down() {
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
                } else if r.fire_at > now && r.fire_at <= until && !s.has_fired(&r.id) {
                    vec![r.fire_at]
                } else {
                    Vec::new()
                };
                // What falls in a pause won't fire: it is skipped when its
                // time comes, so it isn't expected.
                let times: Vec<i64> = times
                    .into_iter()
                    .filter(|t| s.pause_at(r, *t).is_none())
                    .collect();
                let next = self.next_expected(s, r, now).ok();
                out.extend(times.into_iter().map(|t| ExpectedItem {
                    can_close_early: next == Some(t),
                    reminder_id: r.id.clone(),
                    list_id: r.list_id.clone(),
                    priority: r.priority,
                    title: r.title.clone(),
                    note: r.note.clone(),
                    scheduled_at: t,
                    snoozed_until: s.expected_snooze(&r.id, t),
                    expires_at: r.expires_at(t, device),
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
                        list_id: r.list_id.clone(),
                        priority: r.priority,
                        title: r.title.clone(),
                        scheduled_at: o.scheduled_at,
                        closed_at: c.at,
                        kind: c.kind,
                        can_undo: c.kind != ClosingKind::Missed,
                        corrected: !c.replaces.is_empty(),
                        paused: c.paused,
                    })
                })
            })
            .collect();
        earlier
            .sort_by(|a, b| (b.closed_at, &b.occurrence_id).cmp(&(a.closed_at, &a.occurrence_id)));
        // Open occurrences of paused reminders are set aside, quiet.
        let mut paused: Vec<PausedOpen> = Vec::new();
        let mut open: Vec<DueItem> = Vec::new();
        for (_, s) in self.states() {
            for d in s.due(&self.device_tz()) {
                match s
                    .reminders
                    .get(&d.reminder_id)
                    .and_then(|r| s.pause_at(r, now))
                {
                    Some(pause) => paused.push(PausedOpen { item: d, pause }),
                    None => open.push(d),
                }
            }
        }
        self.hold_open(&mut open, now);
        paused.sort_by(|a, b| {
            (a.item.fired_at, &a.item.occurrence_id).cmp(&(b.item.fired_at, &b.item.occurrence_id))
        });
        let (mut overdue, due): (Vec<DueItem>, Vec<DueItem>) =
            open.into_iter().partition(|d| d.overdue_at <= now);
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
            paused,
        }
    }

    // ---- Snooze all and quiet hours (ADR 0013) ----

    /// Snoozes all of the user's reminders, or one list, until `until`: what
    /// is open now and anything that fires before then is quiet, except
    /// Maximum unless `include_maximum`. Occurrences still go overdue on
    /// schedule and last-chance alerts still come. A snooze-all of the same
    /// scope already holding is ended first. It is a personal setting, so it
    /// holds on all of the user's devices. Returns its id.
    pub fn snooze_all(
        &mut self,
        scope: Scope,
        until: i64,
        include_maximum: bool,
        now: i64,
    ) -> Result<String> {
        if until <= now {
            return Err(Error::SnoozeInThePast);
        }
        if let Scope::List(list_id) = &scope {
            if self.state_of(list_id).is_none() {
                return Err(Error::NoList(list_id.clone()));
            }
        }
        let replaced: Vec<String> = self
            .state
            .snooze_alls
            .iter()
            .filter(|r| {
                r.snooze.scope == scope && r.snooze.from <= now && now < r.effective_until()
            })
            .map(|r| r.snooze.id.clone())
            .collect();
        for id in replaced {
            self.record(now, Event::SnoozeAllEnded { snooze_id: id })?;
        }
        let id = Uuid::new_v4().to_string();
        self.record(
            now,
            Event::SnoozeAllStarted {
                snooze: SnoozeAll {
                    id: id.clone(),
                    scope,
                    include_maximum,
                    from: now,
                    until,
                },
            },
        )?;
        Ok(id)
    }

    /// [`snooze_all`](Self::snooze_all) for a length a menu offers.
    pub fn snooze_all_for(
        &mut self,
        scope: Scope,
        choice: SnoozeAllChoice,
        include_maximum: bool,
        now: i64,
    ) -> Result<String> {
        let until = match choice {
            SnoozeAllChoice::Minutes(m) => now + m * 60,
            SnoozeAllChoice::TomorrowMorning => {
                schedule::tomorrow_at(&self.device_tz(), now, TOMORROW_MORNING_HOUR)
                    .ok_or(Error::SnoozeInThePast)?
            }
        };
        self.snooze_all(scope, until, include_maximum, now)
    }

    /// Ends a snooze-all early: everything it held back alerts at its
    /// current level. Ending one that has ended already does nothing.
    pub fn end_snooze_all(&mut self, id: &str, now: i64) -> Result<()> {
        let Some(r) = self.state.snooze_alls.iter().find(|r| r.snooze.id == id) else {
            return Err(Error::NoSnoozeAll(id.to_string()));
        };
        if r.effective_until() <= now {
            return Ok(());
        }
        self.record(
            now,
            Event::SnoozeAllEnded {
                snooze_id: id.to_string(),
            },
        )?;
        Ok(())
    }

    /// What holds now: each snooze-all not yet over, then each stretch of
    /// quiet hours in progress. A snooze-all and a stretch that are the same
    /// thing to the user (the same scope) are both listed.
    pub fn holding(&self, now: i64) -> Vec<SnoozeAllView> {
        let zone = self.device_tz();
        let list_name = |scope: &Scope| match scope {
            Scope::List(l) => self.state_of(l).and_then(|s| s.list_name.clone()),
            Scope::All => None,
        };
        let mut out: Vec<SnoozeAllView> = self
            .state
            .snooze_alls
            .iter()
            .filter(|r| r.snooze.from <= now && now < r.effective_until())
            .map(|r| SnoozeAllView {
                id: Some(r.snooze.id.clone()),
                source: Source::SnoozeAll,
                scope: r.snooze.scope.clone(),
                list_name: list_name(&r.snooze.scope),
                include_maximum: r.snooze.include_maximum,
                from: r.snooze.from,
                until: r.effective_until(),
            })
            .collect();
        out.sort_by_key(|v| (v.from, v.until));
        for q in &self.state.quiet_hours {
            if let Some((from, until)) = q.stretch_at(&zone, now) {
                out.push(SnoozeAllView {
                    id: None,
                    source: Source::QuietHours,
                    scope: q.scope.clone(),
                    list_name: list_name(&q.scope),
                    include_maximum: q.include_maximum,
                    from,
                    until,
                });
            }
        }
        out
    }

    /// The user's quiet hours.
    pub fn quiet_hours(&self) -> Vec<QuietHours> {
        self.state.quiet_hours.clone()
    }

    /// Sets the user's quiet hours: all the rules at once, none to clear
    /// them. A personal setting, so it holds on all of their devices; of two
    /// changes made out of touch, the later clock wins. The times are read in
    /// the zone each device is in.
    pub fn set_quiet_hours(&mut self, rules: Vec<QuietHours>, now: i64) -> Result<()> {
        for r in &rules {
            r.validate().map_err(Error::BadQuietHours)?;
        }
        if rules == self.state.quiet_hours {
            return Ok(());
        }
        let list_id = self.list_id.clone();
        let hlc = self.next_hlc(&list_id, now);
        self.record(now, Event::QuietHoursSet { hlc, rules })?;
        Ok(())
    }

    /// The next moment after `now` that quiet hours start or end, so the
    /// alerter looks again then.
    pub fn next_quiet_boundary(&self, now: i64) -> Option<i64> {
        let zone = self.device_tz();
        self.state
            .quiet_hours
            .iter()
            .filter_map(|q| q.boundary_after(&zone, now))
            .min()
    }

    /// Quiets the open occurrences a snooze-all or quiet hours hold at `now`:
    /// their snooze runs to the end of the hold (and so has no last-chance
    /// alert of its own set at a moment, which is why `snoozed_at` is
    /// `None`; see [`last_chance_at`]).
    fn hold_open(&self, items: &mut [DueItem], now: i64) {
        let zone = self.device_tz();
        for d in items {
            if let Some(h) = self
                .state
                .hold_on(&zone, &d.list_id, d.priority, d.scheduled_at, now)
            {
                if d.snoozed_until.is_none_or(|u| u < h.until) {
                    d.snoozed_until = Some(h.until);
                    d.snoozed_at = None;
                }
            }
        }
    }

    // ---- Pausing (ADR 0011) ----

    /// Sets a reminder aside from now until `until`, or until it is resumed
    /// with `None`. Its instances in the pause are skipped, the history
    /// saying it was the pause, and so is its occurrence that is open now.
    /// Pausing one already paused changes the end and keeps the start.
    pub fn pause_reminder(
        &mut self,
        reminder_id: &str,
        until: Option<i64>,
        now: i64,
    ) -> Result<()> {
        if until.is_some_and(|u| u <= now) {
            return Err(Error::PauseInThePast);
        }
        let list_id = self.list_of_reminder(reminder_id)?;
        self.skip_paused(now)?;
        let state = self.state_of(&list_id).expect("the list is held");
        let current = state.reminders[reminder_id].pause;
        let from = current.filter(|p| p.covers(now)).map_or(now, |p| p.from);
        let pause = Pause { from, until };
        if current != Some(pause) {
            let hlc = self.next_hlc(&list_id, now);
            self.record_in(
                &list_id,
                now,
                Event::ReminderEdited {
                    reminder_id: reminder_id.to_string(),
                    hlc,
                    change: Change::Pause(Some(pause)),
                },
            )?;
        }
        self.skip_open_for_pause(&list_id, Some(reminder_id), now)
    }

    /// Resumes a reminder paused on its own: it fires normally from its next
    /// instance. What fell in the pause stays skipped. (If its list is
    /// paused, it stays set aside until that is resumed.)
    pub fn resume_reminder(&mut self, reminder_id: &str, now: i64) -> Result<()> {
        let list_id = self.list_of_reminder(reminder_id)?;
        if self.state_of(&list_id).expect("the list is held").reminders[reminder_id]
            .pause
            .is_none()
        {
            return Ok(());
        }
        self.skip_paused(now)?;
        let hlc = self.next_hlc(&list_id, now);
        self.record_in(
            &list_id,
            now,
            Event::ReminderEdited {
                reminder_id: reminder_id.to_string(),
                hlc,
                change: Change::Pause(None),
            },
        )?;
        Ok(())
    }

    /// Sets a whole list aside, as [`Core::pause_reminder`] does a reminder:
    /// every reminder in it, including ones moved into it while it is
    /// paused.
    pub fn pause_list(&mut self, list_id: &str, until: Option<i64>, now: i64) -> Result<()> {
        self.check_list(list_id)?;
        if until.is_some_and(|u| u <= now) {
            return Err(Error::PauseInThePast);
        }
        self.skip_paused(now)?;
        let current = self.state_of(list_id).and_then(|s| s.list_pause);
        let from = current.filter(|p| p.covers(now)).map_or(now, |p| p.from);
        let pause = Pause { from, until };
        if current != Some(pause) {
            let hlc = self.next_hlc(list_id, now);
            self.record_in(
                list_id,
                now,
                Event::ListPaused {
                    hlc,
                    pause: Some(pause),
                },
            )?;
        }
        self.skip_open_for_pause(list_id, None, now)
    }

    /// Resumes a paused list.
    pub fn resume_list(&mut self, list_id: &str, now: i64) -> Result<()> {
        self.check_list(list_id)?;
        if self.state_of(list_id).and_then(|s| s.list_pause).is_none() {
            return Ok(());
        }
        self.skip_paused(now)?;
        let hlc = self.next_hlc(list_id, now);
        self.record_in(list_id, now, Event::ListPaused { hlc, pause: None })?;
        Ok(())
    }

    /// Skips the occurrences open now of a reminder (or, with `None`, of
    /// every reminder in the list) that the pause covers: the pause is for
    /// being left alone, and what is open is part of it.
    fn skip_open_for_pause(
        &mut self,
        list_id: &str,
        reminder_id: Option<&str>,
        now: i64,
    ) -> Result<()> {
        let state = self.state_of(list_id).expect("the list is held");
        let open: Vec<(String, String, i64, PauseCause)> = state
            .occurrences
            .values()
            .filter(|o| o.is_open())
            .filter(|o| reminder_id.is_none_or(|r| o.reminder_id == r))
            .filter_map(|o| {
                let r = state.reminders.get(&o.reminder_id)?;
                let cause = state.pause_at(r, now)?;
                Some((o.id.clone(), r.id.clone(), o.scheduled_at, cause))
            })
            .collect();
        for (id, reminder_id, scheduled_at, cause) in open {
            self.record_in(
                list_id,
                now,
                Event::OccurrenceSkippedForPause {
                    occurrence_id: id,
                    reminder_id,
                    scheduled_at,
                    skipped_at: now,
                    until: cause.until,
                    list: cause.list,
                },
            )?;
        }
        Ok(())
    }

    /// Whether a reminder is paused at `now`, by its own pause or its list's.
    /// An event trigger (when they exist) doesn't fire while this is true.
    pub fn is_paused(&self, reminder_id: &str, now: i64) -> bool {
        self.states().any(|(_, s)| s.is_paused(reminder_id, now))
    }

    /// The reminders paused at `now`, by name: the Board's Paused column.
    pub fn paused_reminders(&self, now: i64) -> Vec<PausedReminder> {
        let mut v: Vec<PausedReminder> = self
            .states()
            .flat_map(|(list_id, s)| {
                s.reminders.values().filter_map(move |r| {
                    let pause = s.pause_at(r, now)?;
                    let from = if pause.list {
                        s.list_pause?.from
                    } else {
                        r.pause?.from
                    };
                    Some(PausedReminder {
                        reminder_id: r.id.clone(),
                        list_id: list_id.to_string(),
                        title: r.title.clone(),
                        priority: r.priority,
                        pause,
                        from,
                        has_open: s
                            .occurrences
                            .values()
                            .any(|o| o.reminder_id == r.id && o.is_open()),
                    })
                })
            })
            .collect();
        v.sort_by(|a, b| (&a.title, &a.reminder_id).cmp(&(&b.title, &b.reminder_id)));
        v
    }

    // ---- Lists, moving and deleting (ADR 0009) ----

    /// Makes a new list, which gets its own stream and so its own key. Only
    /// this device knows it until the sync layer registers it with the server.
    /// Returns its id.
    pub fn create_list(&mut self, name: &str, colour: Option<&str>, now: i64) -> Result<String> {
        let name = check_list_name(name)?;
        let colour = colour.map(check_colour).transpose()?;
        let list_id = Uuid::new_v4().to_string();
        self.record_in(&list_id, now, Event::ListNamed { name })?;
        if let Some(colour) = colour {
            self.record_in(&list_id, now, Event::ListColoured { colour })?;
        }
        Ok(list_id)
    }

    /// Renames a list. The personal list keeps its name.
    pub fn rename_list(&mut self, list_id: &str, name: &str, now: i64) -> Result<()> {
        self.check_list(list_id)?;
        if list_id == self.list_id {
            return Err(Error::PersonalList);
        }
        let name = check_list_name(name)?;
        if self.state_of(list_id).and_then(|s| s.list_name.as_deref()) == Some(name.as_str()) {
            return Ok(());
        }
        self.record_in(list_id, now, Event::ListNamed { name })?;
        Ok(())
    }

    /// Gives a list (the personal one too) a colour, `#rrggbb`.
    pub fn colour_list(&mut self, list_id: &str, colour: &str, now: i64) -> Result<()> {
        self.check_list(list_id)?;
        let colour = check_colour(colour)?;
        if self
            .state_of(list_id)
            .and_then(|s| s.list_colour.as_deref())
            == Some(colour.as_str())
        {
            return Ok(());
        }
        self.record_in(list_id, now, Event::ListColoured { colour })?;
        Ok(())
    }

    /// Deletes a list that holds no reminders. The personal list can't be
    /// deleted. (Its stream stays on the server until retention limits can
    /// remove it; nothing in it is shown again.)
    pub fn delete_list(&mut self, list_id: &str, now: i64) -> Result<()> {
        self.check_list(list_id)?;
        if list_id == self.list_id {
            return Err(Error::PersonalList);
        }
        if !self
            .state_of(list_id)
            .is_some_and(|s| s.reminders.is_empty())
        {
            return Err(Error::ListNotEmpty);
        }
        self.record_in(list_id, now, Event::ListDeleted)?;
        Ok(())
    }

    /// Moves a reminder to another list. It keeps its settings, its history
    /// and its occurrences, open or closed, under the same ids, so nothing
    /// fires twice or is forgotten. Of two moves of the same reminder on
    /// devices out of touch, the later wins.
    pub fn move_reminder(&mut self, reminder_id: &str, to_list_id: &str, now: i64) -> Result<()> {
        let from = self.list_of_reminder(reminder_id)?;
        self.check_list(to_list_id)?;
        if from == to_list_id {
            return Err(Error::SameList);
        }
        let seen = self
            .states()
            .map(|(_, s)| s.latest_hlc())
            .max()
            .cloned()
            .unwrap_or_default();
        let hlc = Hlc::next(now, &self.device_id, &seen);
        self.record_in(
            to_list_id,
            now,
            Event::ReminderMovedIn {
                reminder_id: reminder_id.to_string(),
                from_list_id: from,
                hlc,
            },
        )?;
        // It now lives in the other list, which takes it from this one.
        self.rebuild()
    }

    /// Deletes a reminder but keeps its history, marked deleted. It never
    /// fires or alerts again, here or on any device that gets this change,
    /// and an edit, move or firing made meanwhile on another device doesn't
    /// bring it back.
    pub fn delete_reminder(&mut self, reminder_id: &str, now: i64) -> Result<()> {
        let list_id = self.list_of_reminder(reminder_id)?;
        self.record_in(
            &list_id,
            now,
            Event::ReminderDeleted {
                reminder_id: reminder_id.to_string(),
            },
        )?;
        Ok(())
    }

    /// Deletes a reminder with its history (also one deleted before, with its
    /// history kept). Every device that gets this change forgets the
    /// reminder and removes what it stored of it. Copies on devices that
    /// have not synced yet, and the server's encrypted copy of the events,
    /// are not recalled.
    pub fn purge_reminder(&mut self, reminder_id: &str, now: i64) -> Result<()> {
        let list_id = self
            .states()
            .find(|(_, s)| s.knows(reminder_id))
            .map(|(id, _)| id.to_string())
            .ok_or_else(|| Error::NoReminder(reminder_id.to_string()))?;
        self.record_in(
            &list_id,
            now,
            Event::ReminderPurged {
                reminder_id: reminder_id.to_string(),
            },
        )?;
        self.store.redact_reminder(reminder_id)?;
        self.rebuild()
    }

    /// How many events this device has stored that mention the reminder,
    /// apart from the ones that record its purging. After a purge, none.
    pub fn stored_events_of(&self, reminder_id: &str) -> Result<usize> {
        self.store.count_mentioning(reminder_id)
    }

    /// Reminders deleted with their history kept, newest deletion first.
    pub fn deleted_reminders(&self) -> Vec<DeletedReminder> {
        let mut v: Vec<DeletedReminder> = self
            .states()
            .flat_map(|(list_id, s)| {
                s.deleted_reminders()
                    .into_iter()
                    .map(move |(r, t)| DeletedReminder {
                        reminder_id: r.id.clone(),
                        list_id: list_id.to_string(),
                        title: r.title.clone(),
                        deleted_at: t.at,
                        deleted_by: t.by.clone(),
                    })
            })
            .collect();
        v.sort_by(|a, b| (b.deleted_at, &b.reminder_id).cmp(&(a.deleted_at, &a.reminder_id)));
        v
    }

    /// The sidebar's list and priority checkboxes, as this device remembers
    /// them. Nothing is hidden until the user hides it.
    pub fn filters(&self) -> Filters {
        self.device_setting(FILTERS)
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default()
    }

    /// Remembers the sidebar's checkboxes on this device. They decide only
    /// what the window lists; hiding a list doesn't stop its alerts.
    pub fn set_filters(&self, filters: &Filters) -> Result<()> {
        self.set_device_setting(FILTERS, &serde_json::to_string(filters)?)
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

    /// The loudest alert style this device uses. An alarm, which caps
    /// nothing, until it is set. Stays on this device (spec: Sync → What
    /// syncs where).
    pub fn loudest_alert(&self) -> LoudestAlert {
        let style = self
            .device_setting(LOUDEST_STYLE)
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value(serde_json::Value::String(v)).ok())
            .unwrap_or(AlertStyle::Alarm);
        let caps_maximum = matches!(
            self.device_setting(LOUDEST_CAPS_MAXIMUM)
                .ok()
                .flatten()
                .as_deref(),
            Some("1")
        );
        LoudestAlert {
            style,
            caps_maximum,
        }
    }

    /// Caps the loudest style this device uses. Louder alerts are downgraded
    /// on this device only; Maximum still gets through unless
    /// `caps_maximum`. Nothing is written to any list: it never syncs.
    pub fn set_loudest_alert(&self, loudest: LoudestAlert) -> Result<()> {
        let name = match serde_json::to_value(loudest.style)? {
            serde_json::Value::String(s) => s,
            _ => return Ok(()),
        };
        self.set_device_setting(LOUDEST_STYLE, &name)?;
        self.set_device_setting(
            LOUDEST_CAPS_MAXIMUM,
            if loudest.caps_maximum { "1" } else { "0" },
        )
    }

    /// "Quiet this device until…" if it is in force at `now`.
    pub fn device_quiet(&self, now: i64) -> Option<DeviceQuiet> {
        self.device_setting(QUIET_DEVICE)
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str::<DeviceQuiet>(&v).ok())
            .filter(|q| q.until > now)
    }

    /// Both per-device limits, for the alerter.
    pub fn device_limits(&self, now: i64) -> DeviceLimits {
        DeviceLimits {
            loudest: self.loudest_alert(),
            quiet: self.device_quiet(now),
        }
    }

    /// Quiets everything on this device until `until`: alerts here are silent
    /// (Maximum left out unless `include_maximum`). It isn't a snooze: no
    /// occurrence records it, it never syncs, and other devices still alert.
    pub fn quiet_device(&self, until: i64, include_maximum: bool, now: i64) -> Result<()> {
        if until <= now {
            return Err(Error::SnoozeInThePast);
        }
        self.set_device_setting(
            QUIET_DEVICE,
            &serde_json::to_string(&DeviceQuiet {
                until,
                include_maximum,
            })?,
        )
    }

    /// [`quiet_device`](Self::quiet_device) for a length a menu offers.
    pub fn quiet_device_for(
        &self,
        choice: SnoozeAllChoice,
        include_maximum: bool,
        now: i64,
    ) -> Result<()> {
        let until = match choice {
            SnoozeAllChoice::Minutes(m) => now + m * 60,
            SnoozeAllChoice::TomorrowMorning => {
                schedule::tomorrow_at(&self.device_tz(), now, TOMORROW_MORNING_HOUR)
                    .ok_or(Error::SnoozeInThePast)?
            }
        };
        self.quiet_device(until, include_maximum, now)
    }

    /// Ends "Quiet this device" early: alerts here come at their current
    /// level.
    pub fn end_quiet_device(&self) -> Result<()> {
        self.set_device_setting(QUIET_DEVICE, "")
    }

    /// Records that this device is `portable` or stationary, in the personal
    /// list beside its name. Recorded only when it changes.
    pub fn set_portable(&mut self, portable: bool, now: i64) -> Result<()> {
        if self.state.device_portable.get(&self.device_id) == Some(&portable) {
            return Ok(());
        }
        self.record(now, Event::DevicePortable { portable })?;
        Ok(())
    }

    /// Whether `device_id` says it is portable, if it has said.
    pub fn device_portable(&self, device_id: &str) -> Option<bool> {
        self.state.device_portable.get(device_id).copied()
    }

    /// This device's name as recorded in the personal list.
    pub fn own_device_name(&self) -> Option<&str> {
        self.device_name(&self.device_id)
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
                let mut due: Vec<DueItem> = self
                    .states()
                    .flat_map(|(_, s)| s.due(&self.device_tz()))
                    .collect();
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
                                list_id: r.list_id.clone(),
                                priority: r.priority,
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

/// Settles what the lists' streams say of reminders that were deleted with
/// their history or moved between lists, so that each reminder is in exactly
/// one list and every device that has the same events agrees which (ADR 0009).
fn settle(all: &mut BTreeMap<String, State>) {
    // Deleted with their history: nothing of them is kept, wherever it is.
    let purged: BTreeSet<String> = all
        .values()
        .flat_map(|s| s.purged.iter().cloned())
        .collect();
    for s in all.values_mut() {
        for id in &purged {
            s.purged.insert(id.clone());
            s.drop_reminder(id);
        }
    }
    // A reminder is where its latest move put it, or where it was made.
    let moved: BTreeSet<String> = all
        .values()
        .flat_map(|s| s.moves_in.iter().map(|m| m.reminder_id.clone()))
        .collect();
    for id in moved {
        let Some((_, current)) = all
            .iter()
            .filter_map(|(list, s)| {
                s.moved_in_at(&id)
                    .cloned()
                    .or_else(|| s.knows(&id).then(Hlc::default))
                    .map(|at| (at, list.clone()))
            })
            .max()
        else {
            continue;
        };
        let mut here = all.remove(&current).expect("the list was just seen");
        for (list, s) in all.iter() {
            if s.knows(&id) && *list != current {
                // Whatever was done to it where it was, as well.
                here.absorb(s, &id, &current);
            }
        }
        here.replay_pending();
        all.insert(current.clone(), here);
        for (list, s) in all.iter_mut() {
            if *list != current {
                s.drop_reminder(&id);
            }
        }
    }
}

/// A list's name, trimmed.
fn check_list_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::EmptyListName);
    }
    Ok(name.chars().take(80).collect())
}

/// A colour as `#rrggbb`, in lower case.
fn check_colour(colour: &str) -> Result<String> {
    let c = colour.trim().to_lowercase();
    let ok = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
    if ok {
        Ok(c)
    } else {
        Err(Error::BadColour(colour.to_string()))
    }
}

/// Checks a delay can be used.
fn check_delay(d: &Delay) -> Result<()> {
    if matches!(d, Delay::After(n) if *n < 0) {
        return Err(Error::BadDuration);
    }
    d.validate().map_err(Error::BadDelay)
}

/// Checks sun-event triggers can be used.
fn check_suns(suns: &[SunTrigger]) -> Result<()> {
    suns.iter()
        .try_for_each(|s| s.validate().map_err(Error::BadSunEvent))
}

/// Checks time-based conditions can be used.
fn check_conditions(conditions: &[Condition]) -> Result<()> {
    conditions
        .iter()
        .try_for_each(|c| c.validate().map_err(Error::BadCondition))
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
