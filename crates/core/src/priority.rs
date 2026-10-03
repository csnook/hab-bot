//! Priorities: how insistently a reminder's open occurrences alert, and how
//! long one stays due before it goes overdue (spec: Alerts → Priorities).
//!
//! Only the built-in priorities exist so far. Each carries every setting in
//! the spec's table, so the alert tickets read them from here.

use serde::{Deserialize, Serialize};

/// A built-in priority. The order is the order of insistence: sorting by it
/// puts Maximum last.
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

/// How loud one alert is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertStyle {
    /// Only listed (on desktop).
    Silent,
    Gentle,
    /// Gentle, repeated each interval.
    Insistent,
    /// Ringing until someone acts.
    Alarm,
}

/// "After `after` seconds overdue, switch to `style`".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EscalationStep {
    pub after: i64,
    pub style: AlertStyle,
}

/// Everything a priority sets. Times are in seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrioritySettings {
    pub due_style: AlertStyle,
    /// How long an occurrence is due before it goes overdue; also the snooze
    /// length while due, how long a wait lasts and how long a later event
    /// joins the occurrence. 0 means overdue at once.
    pub due_interval: i64,
    /// The style once overdue, escalating in steps: the first starts at 0.
    pub overdue_steps: Vec<EscalationStep>,
    /// Once overdue: the snooze length, the repeat interval and the quiet
    /// period after acknowledging.
    pub overdue_interval: i64,
    /// How long an alarm rings; `None` is the whole interval, meaning until
    /// someone acts.
    pub ring_duration: Option<i64>,
    /// How long the check with the server before each alert may hold it up;
    /// `None` is no wait.
    pub server_wait: Option<i64>,
    /// Whether a swipe on Android is allowed (as a snooze).
    pub swipeable: bool,
    pub breaks_do_not_disturb: bool,
}

impl PrioritySettings {
    /// The style an occurrence that went (or goes) overdue at `overdue_at`
    /// alerts in at `now`: the due style until then, then the step with the
    /// largest `after` that has been reached. Steps count from `overdue_at`.
    pub fn style_at(&self, overdue_at: i64, now: i64) -> AlertStyle {
        if now < overdue_at {
            return self.due_style;
        }
        let overdue_for = now - overdue_at;
        self.overdue_steps
            .iter()
            .filter(|s| s.after <= overdue_for)
            .max_by_key(|s| s.after)
            .map_or(self.due_style, |s| s.style)
    }

    /// How often an insistent alert repeats: the overdue interval, which is
    /// also the snooze length. Never less than a minute, so a priority with
    /// an interval of 0 can't make the desktop spin.
    pub fn repeat_every(&self) -> i64 {
        self.overdue_interval.max(MINUTE)
    }
}

impl PrioritySettings {
    /// How long one tap on Snooze quiets an occurrence: the due interval
    /// while it is due, the overdue interval once it is overdue. A priority
    /// that is overdue at once (a due interval of 0) snoozes by the overdue
    /// interval too.
    pub fn snooze_length(&self, overdue: bool) -> i64 {
        if overdue || self.due_interval == 0 {
            self.repeat_every()
        } else {
            self.due_interval
        }
    }

    /// How long an alarm rings from the moment it alerts: its ring duration,
    /// or `None` for until someone acts (or the next repeat takes over).
    pub fn rings_for(&self) -> Option<i64> {
        self.ring_duration
    }
}

const MINUTE: i64 = 60;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

impl Priority {
    /// The five built-ins, least insistent first.
    pub const BUILT_IN: [Priority; 5] = [
        Priority::Minimum,
        Priority::Low,
        Priority::Medium,
        Priority::High,
        Priority::Maximum,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Priority::Minimum => "Minimum",
            Priority::Low => "Low",
            Priority::Medium => "Medium",
            Priority::High => "High",
            Priority::Maximum => "Maximum",
        }
    }

    pub fn settings(self) -> PrioritySettings {
        let step = |after, style| EscalationStep { after, style };
        use AlertStyle::*;
        match self {
            Priority::Minimum => PrioritySettings {
                due_style: Silent,
                due_interval: DAY,
                overdue_steps: vec![step(0, Silent)],
                overdue_interval: DAY,
                ring_duration: None,
                server_wait: Some(60),
                swipeable: true,
                breaks_do_not_disturb: false,
            },
            Priority::Low => PrioritySettings {
                due_style: Gentle,
                due_interval: DAY,
                overdue_steps: vec![step(0, Silent)],
                overdue_interval: DAY,
                ring_duration: None,
                server_wait: Some(60),
                swipeable: true,
                breaks_do_not_disturb: false,
            },
            Priority::Medium => PrioritySettings {
                due_style: Gentle,
                due_interval: HOUR,
                overdue_steps: vec![step(0, Insistent), step(HOUR, Alarm)],
                overdue_interval: 10 * MINUTE,
                ring_duration: None,
                server_wait: Some(60),
                swipeable: true,
                breaks_do_not_disturb: false,
            },
            Priority::High => PrioritySettings {
                due_style: Alarm,
                due_interval: 0,
                overdue_steps: vec![step(0, Alarm)],
                overdue_interval: 10 * MINUTE,
                ring_duration: None,
                server_wait: Some(60),
                swipeable: false,
                breaks_do_not_disturb: false,
            },
            Priority::Maximum => PrioritySettings {
                due_style: Alarm,
                due_interval: 0,
                overdue_steps: vec![step(0, Alarm)],
                overdue_interval: 10 * MINUTE,
                ring_duration: None,
                server_wait: None,
                swipeable: false,
                breaks_do_not_disturb: true,
            },
        }
    }
}

/// A built-in priority as the settings dialog lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PriorityInfo {
    pub priority: Priority,
    pub name: &'static str,
    pub settings: PrioritySettings,
}

/// The built-in priorities with their settings, least insistent first.
pub fn built_in_priorities() -> Vec<PriorityInfo> {
    Priority::BUILT_IN
        .iter()
        .map(|p| PriorityInfo {
            priority: *p,
            name: p.name(),
            settings: p.settings(),
        })
        .collect()
}
