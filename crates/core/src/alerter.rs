//! Deciding what a device alerts about, and when (spec: Alerts → Alert styles,
//! Priorities, Do Not Disturb).
//!
//! The [`Alerter`] is the platform-independent half of alerting. Each pass it
//! looks at the open occurrences, works out the style each should alert in
//! now, and returns [`Command`]s for the platform to carry out: show or
//! replace a notification, or close one. The Linux half (D-Bus
//! notifications) lives in the app; Android will have its own.
//!
//! What it decides, per open occurrence:
//! - the style: the priority's due style, or once overdue the escalation step
//!   with the largest `after` reached (steps count from the overdue time);
//! - downgraded to silent while the notification server is inhibited (Do Not
//!   Disturb), unless the priority breaks it; the occurrence then catches up
//!   at its current level once the inhibition ends;
//! - nothing while the occurrence is snoozed or acknowledged; a snooze that
//!   ends alerts again at the current level;
//! - an alert at first sight, at each change of style, and, for insistent,
//!   repeated every interval until the occurrence closes.
//!
//! The first alert and each change of style are written to the history with
//! the device that alerted; repeats and catching up at the same style are not.
//!
//! Alarm is #42's. This module still decides *when* an alarm alerts, and
//! reports it as [`Notification::style`] `== Alarm`; until the alarm exists
//! the notification it carries is the insistent one, repeated each interval.

use std::collections::{HashMap, HashSet};

use crate::core::Core;
use crate::priority::AlertStyle;
use crate::Result;

/// The key of the Done button.
pub const ACTION_DONE: &str = "done";
/// The key of the Skip button.
pub const ACTION_SKIP: &str = "skip";
/// The key freedesktop.org servers send when the notification itself is
/// clicked.
pub const ACTION_OPEN: &str = "default";

/// The freedesktop.org urgency of a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

/// A notification to show. The platform replaces the one it already shows for
/// the same occurrence, so a repeat pops up again instead of piling up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub occurrence_id: String,
    pub title: String,
    pub body: String,
    /// The style this alert is in (after any Do Not Disturb downgrade).
    pub style: AlertStyle,
    pub urgency: Urgency,
    /// A name from the sound theme, played by the notification server.
    pub sound: Option<&'static str>,
    /// How long the server shows it: `-1` its default, `0` never expires.
    pub timeout_ms: i32,
    /// Buttons as (key, label), after the click action.
    pub actions: Vec<(&'static str, &'static str)>,
}

/// What the platform should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Show(Notification),
    /// The occurrence closed or was snoozed: take its notification down.
    Close {
        occurrence_id: String,
    },
}

/// The sound-theme name for gentle and insistent alerts.
const SOUND: &str = "message-new-instant";
/// How long a gentle notification stays up before it settles into the
/// server's history.
const GENTLE_TIMEOUT_MS: i32 = 10_000;

impl Notification {
    fn new(occurrence_id: &str, title: &str, style: AlertStyle, overdue: bool) -> Self {
        let (urgency, sound, timeout_ms) = match style {
            AlertStyle::Silent => (Urgency::Low, None, -1),
            // Gentle, insistent and (until #42) alarm are all normal
            // notifications with a themed sound that time out.
            AlertStyle::Gentle | AlertStyle::Insistent | AlertStyle::Alarm => {
                (Urgency::Normal, Some(SOUND), GENTLE_TIMEOUT_MS)
            }
        };
        Notification {
            occurrence_id: occurrence_id.to_string(),
            title: title.to_string(),
            body: if overdue { "Overdue" } else { "Due" }.to_string(),
            style,
            urgency,
            sound,
            timeout_ms,
            actions: vec![(ACTION_DONE, "Done"), (ACTION_SKIP, "Skip")],
        }
    }
}

/// What one pass decided.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pass {
    pub commands: Vec<Command>,
    /// When the next pass is due even if nothing changes: an escalation, a
    /// repeat, or the end of a snooze.
    pub next_at: Option<i64>,
}

#[derive(Debug, Default)]
struct Tracked {
    /// The style of the alert currently standing, if this run alerted and
    /// hasn't since gone quiet.
    standing: Option<AlertStyle>,
    /// When it last alerted, for repeats.
    last_at: i64,
    /// The style last written to the history, so a repeat or a catch-up at
    /// the same style isn't recorded again.
    recorded: Option<AlertStyle>,
}

/// The per-device alert planner. It remembers only what it has alerted
/// about since the app started.
#[derive(Debug, Default)]
pub struct Alerter {
    tracked: HashMap<String, Tracked>,
}

impl Alerter {
    pub fn new() -> Self {
        Self::default()
    }

    /// One pass at `now`. `inhibited` is the notification server's Do Not
    /// Disturb state. Writes the alerts that belong in the history.
    pub fn pass(&mut self, core: &mut Core, now: i64, inhibited: bool) -> Result<Pass> {
        let inbox = core.inbox(now);
        let open: Vec<_> = inbox.overdue.iter().chain(inbox.due.iter()).collect();
        let mut out = Pass::default();
        let soonest = |t: i64, out: &mut Pass| {
            if t > now {
                out.next_at = Some(out.next_at.map_or(t, |n| n.min(t)));
            }
        };

        // Occurrences that closed: their notifications go.
        let ids: HashSet<&str> = open.iter().map(|d| d.occurrence_id.as_str()).collect();
        let mut gone: Vec<String> = self
            .tracked
            .keys()
            .filter(|k| !ids.contains(k.as_str()))
            .cloned()
            .collect();
        gone.sort();
        for id in gone {
            if let Some(t) = self.tracked.remove(&id) {
                if t.standing.is_some() {
                    out.commands.push(Command::Close { occurrence_id: id });
                }
            }
        }

        for d in open {
            let settings = d.priority.settings();
            let tracked = self
                .tracked
                .entry(d.occurrence_id.clone())
                .or_insert_with(|| Tracked {
                    recorded: core.last_alert_style(&d.occurrence_id),
                    ..Default::default()
                });
            // The next escalation step is a moment to look again.
            for step in &settings.overdue_steps {
                soonest(d.overdue_at + step.after, &mut out);
            }
            soonest(d.overdue_at, &mut out);

            // Snoozed or acknowledged: quiet. A snooze that ends alerts again.
            let snoozed = d.snoozed_until.filter(|&u| u > now);
            if snoozed.is_some() || d.acknowledged {
                if tracked.standing.take().is_some() {
                    out.commands.push(Command::Close {
                        occurrence_id: d.occurrence_id.clone(),
                    });
                }
                if let Some(u) = snoozed {
                    soonest(u, &mut out);
                }
                continue;
            }

            let wanted = settings.style_at(d.overdue_at, now);
            let style = if inhibited && !settings.breaks_do_not_disturb {
                AlertStyle::Silent
            } else {
                wanted
            };
            let repeats = matches!(style, AlertStyle::Insistent | AlertStyle::Alarm);
            let repeat_at = tracked.last_at + settings.repeat_every();
            let alert = match tracked.standing {
                None => true,
                Some(standing) if standing != style => true,
                Some(_) => repeats && now >= repeat_at,
            };
            if alert {
                if tracked.recorded != Some(style) {
                    core.record_alert(&d.occurrence_id, style, now)?;
                    tracked.recorded = Some(style);
                }
                tracked.standing = Some(style);
                tracked.last_at = now;
                out.commands.push(Command::Show(Notification::new(
                    &d.occurrence_id,
                    &d.title,
                    style,
                    d.overdue_at <= now,
                )));
            }
            if repeats {
                soonest(tracked.last_at + settings.repeat_every(), &mut out);
            }
        }
        Ok(out)
    }
}
