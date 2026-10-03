//! The five built-in priorities and what each is made of (spec: Alerts → Priorities).

use crate::Millis;
use serde::{Deserialize, Serialize};

const MINUTE: Millis = 60_000;
const HOUR: Millis = 60 * MINUTE;
const DAY: Millis = 24 * HOUR;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Minimum,
    Low,
    #[default]
    Medium,
    High,
    Maximum,
}

/// How an alert gets attention. How each is delivered comes with the alert tickets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertStyle {
    Silent,
    Gentle,
    Insistent,
    Alarm,
}

/// "After this long overdue, switch to this style."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Escalation {
    pub after: Millis,
    pub style: AlertStyle,
}

/// Every setting a priority carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrioritySettings {
    pub priority: Priority,
    pub name: &'static str,
    pub due_style: AlertStyle,
    /// The style once overdue, escalating in steps; the last step repeats.
    pub overdue: Vec<Escalation>,
    /// While due: the snooze length, how long until the occurrence goes overdue, how long
    /// a wait lasts by default and how long a later event joins it. 0 is overdue at once.
    pub due_interval: Millis,
    /// Once overdue: the snooze length, the repeat interval and the quiet period after
    /// acknowledging.
    pub overdue_interval: Millis,
    /// How long an alarm rings; `None` is the whole interval, until someone acts.
    pub ring_duration: Option<Millis>,
    /// How long the check with the server before each alert may hold it up.
    pub server_wait: Option<Millis>,
    /// Whether a notification can be swiped away on Android (a swipe is a snooze).
    pub swipeable: bool,
    pub breaks_do_not_disturb: bool,
}

pub const ALL: [Priority; 5] = [
    Priority::Minimum,
    Priority::Low,
    Priority::Medium,
    Priority::High,
    Priority::Maximum,
];

impl Priority {
    pub fn settings(self) -> PrioritySettings {
        use AlertStyle::*;
        let step = |after, style| Escalation { after, style };
        let (name, due_style, overdue, due_interval, overdue_interval) = match self {
            Priority::Minimum => ("Minimum", Silent, vec![step(0, Silent)], DAY, DAY),
            Priority::Low => ("Low", Gentle, vec![step(0, Silent)], DAY, DAY),
            Priority::Medium => (
                "Medium",
                Gentle,
                vec![step(0, Insistent), step(HOUR, Alarm)],
                HOUR,
                10 * MINUTE,
            ),
            Priority::High => ("High", Alarm, vec![step(0, Alarm)], 0, 10 * MINUTE),
            Priority::Maximum => ("Maximum", Alarm, vec![step(0, Alarm)], 0, 10 * MINUTE),
        };
        PrioritySettings {
            priority: self,
            name,
            due_style,
            overdue,
            due_interval,
            overdue_interval,
            ring_duration: None,
            server_wait: if self == Priority::Maximum {
                None
            } else {
                Some(60_000)
            },
            swipeable: self <= Priority::Medium,
            breaks_do_not_disturb: self == Priority::Maximum,
        }
    }

    /// When an occurrence scheduled at `scheduled_at` goes overdue: after the due interval,
    /// counted from the scheduled time.
    pub fn overdue_at(self, scheduled_at: Millis) -> Millis {
        scheduled_at + self.settings().due_interval
    }
}

pub fn all_settings() -> Vec<PrioritySettings> {
    ALL.iter().map(|p| p.settings()).collect()
}
