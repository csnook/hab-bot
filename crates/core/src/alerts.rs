//! What each open occurrence should be alerting with right now (spec: Alerts).
//!
//! The [`AlertEngine`] lives on a device. It decides when to alert and in which style, and
//! the platform delivers that: D-Bus notifications on Linux, Kotlin on Android. Repeat
//! timing is the device's own and isn't synced; the history records only the first alert
//! and each change of style, with the device that alerted.

use crate::{AlertStyle, Core, Millis, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// How long before a known expiry the last-chance alert comes.
pub const LAST_CHANCE: Millis = 10 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    /// The occurrence's first alert on this device.
    First,
    /// The style changed: it went overdue, or escalated, or Do Not Disturb began or ended.
    StyleChange,
    /// An insistent alert or alarm repeating at its interval.
    Repeat,
    /// A known expiry falls inside the snooze, and is 10 minutes away.
    LastChance,
}

/// Something for the platform to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Alert {
    pub occurrence_id: String,
    pub title: String,
    pub kind: AlertKind,
    /// The style to deliver now, after any Do Not Disturb downgrade.
    pub style: AlertStyle,
    pub overdue: bool,
    /// For a last-chance alert: when the occurrence expires.
    pub expires_at: Option<Millis>,
}

/// What a poll found: alerts to show, and notifications to take down because their
/// occurrence closed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Poll {
    pub alerts: Vec<Alert>,
    pub dismissed: Vec<String>,
}

struct Shown {
    /// The style last delivered, after any downgrade.
    effective: AlertStyle,
    /// The style the priority calls for, which history records changes of.
    nominal: AlertStyle,
    at: Millis,
}

/// One device's alerting state.
pub struct AlertEngine {
    device: String,
    shown: HashMap<String, Shown>,
    last_chance_sent: HashSet<String>,
}

/// The style a priority calls for at `now`: its due style until the occurrence goes
/// overdue, then its escalation steps counted from the overdue time.
fn nominal_style(o: &crate::Occurrence, now: Millis) -> AlertStyle {
    let settings = o.priority.settings();
    if now < o.overdue_at {
        return settings.due_style;
    }
    let overdue_for = now - o.overdue_at;
    settings
        .overdue
        .iter()
        .rev()
        .find(|step| step.after <= overdue_for)
        .map_or(settings.due_style, |step| step.style)
}

impl AlertEngine {
    pub fn new(device: &str) -> AlertEngine {
        AlertEngine {
            device: device.to_string(),
            shown: HashMap::new(),
            last_chance_sent: HashSet::new(),
        }
    }

    /// Looks at every open occurrence and says what to show now. `dnd` is whether the
    /// system's Do Not Disturb is on: priorities that don't break it are downgraded to
    /// silent, and catch up at their current level afterwards. History is recorded here.
    pub fn poll(&mut self, core: &mut Core, now: Millis, dnd: bool) -> Result<Poll> {
        let mut poll = Poll::default();
        let open = core.open_occurrences();

        let open_ids: Vec<&String> = open.iter().map(|o| &o.id).collect();
        let gone: Vec<String> = self
            .shown
            .keys()
            .filter(|id| !open_ids.contains(id))
            .cloned()
            .collect();
        for id in gone {
            self.shown.remove(&id);
            self.last_chance_sent.remove(&id);
            poll.dismissed.push(id);
        }

        for o in open {
            let settings = o.priority.settings();
            let nominal = nominal_style(&o, now);
            let snoozed = o.snoozed_until.is_some_and(|until| now < until);
            let dnd_here = dnd && !settings.breaks_do_not_disturb;
            let effective = if snoozed || dnd_here {
                AlertStyle::Silent
            } else {
                nominal
            };

            // A snooze can't silently turn into a miss: a known expiry inside it gets a
            // last-chance alert, in the due style but never quieter than gentle.
            if let (true, Some(expires), Some(until)) = (snoozed, o.expires_at, o.snoozed_until) {
                if expires <= until
                    && now >= expires.saturating_sub(LAST_CHANCE)
                    && self.last_chance_sent.insert(o.id.clone())
                {
                    let style = if dnd_here {
                        AlertStyle::Silent
                    } else {
                        settings.due_style.max(AlertStyle::Gentle)
                    };
                    poll.alerts.push(Alert {
                        occurrence_id: o.id.clone(),
                        title: o.title.clone(),
                        kind: AlertKind::LastChance,
                        style,
                        overdue: now >= o.overdue_at,
                        expires_at: Some(expires),
                    });
                }
            }
            let overdue = now >= o.overdue_at;
            let kind = match self.shown.get(&o.id) {
                None => Some(AlertKind::First),
                Some(s) if s.effective != effective => Some(AlertKind::StyleChange),
                Some(s) if repeats(effective) && now >= s.at + settings.overdue_interval.max(1) => {
                    Some(AlertKind::Repeat)
                }
                Some(_) => None,
            };
            let Some(kind) = kind else { continue };

            // History: the first alert and each change of the priority's style, not repeats
            // and not Do Not Disturb downgrades.
            let previous_nominal = self.shown.get(&o.id).map(|s| s.nominal);
            if previous_nominal != Some(nominal) {
                core.record_alert(&o.id, nominal, &self.device, now)?;
            }
            self.shown.insert(
                o.id.clone(),
                Shown {
                    effective,
                    nominal,
                    at: now,
                },
            );
            poll.alerts.push(Alert {
                occurrence_id: o.id,
                title: o.title,
                kind,
                style: effective,
                overdue,
                expires_at: None,
            });
        }
        Ok(poll)
    }

    /// The next moment `poll` could have something to say, for scheduling a wake-up.
    pub fn next_wake(&self, core: &Core, now: Millis) -> Option<Millis> {
        let mut soonest: Option<Millis> = None;
        let mut consider = |t: Millis| soonest = Some(soonest.map_or(t, |s| s.min(t)));
        for o in core.open_occurrences() {
            let settings = o.priority.settings();
            match self.shown.get(&o.id) {
                None => consider(now),
                Some(shown) => {
                    if repeats(shown.effective) {
                        consider(shown.at + settings.overdue_interval.max(1));
                    }
                }
            }
            if now < o.overdue_at {
                consider(o.overdue_at);
            }
            if let Some(until) = o.snoozed_until.filter(|u| *u > now) {
                consider(until);
                if let Some(expires) = o.expires_at.filter(|e| *e <= until) {
                    if !self.last_chance_sent.contains(&o.id) {
                        consider(expires.saturating_sub(LAST_CHANCE).max(now));
                    }
                }
            }
            for step in &settings.overdue {
                let at = o.overdue_at + step.after;
                if at > now {
                    consider(at);
                }
            }
        }
        soonest
    }
}

/// Insistent alerts and alarms repeat until someone acts; silent and gentle ones don't.
fn repeats(style: AlertStyle) -> bool {
    matches!(style, AlertStyle::Insistent | AlertStyle::Alarm)
}
