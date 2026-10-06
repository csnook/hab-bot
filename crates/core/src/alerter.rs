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
//! - capped to the device's loudest style and made silent while the device
//!   is quiet (`Core::device_limits`, spec: Alerts → Settings per device),
//!   Maximum exempt from each unless included; this is the device's own
//!   downgrade, never a snooze, and it never reaches another device;
//! - nothing while the occurrence is snoozed or acknowledged; a snooze that
//!   ends alerts again at the current level;
//! - an alert at first sight, at each change of style, and, for insistent,
//!   repeated every interval until the occurrence closes.
//!
//! The style recorded in the history is the one this device alerted in, after
//! Do Not Disturb and the device's limits: it is the history of this device's
//! alerts. The last-chance alert is limited the same way (a quiet device
//! shows it silently), and the server check is made as for any alert.
//!
//! The first alert and each change of style are written to the history with
//! the device that alerted; repeats and catching up at the same style are not.
//!
//! An alarm is a critical notification that never times out, with the app's
//! own looping sound and its alarm window (the platform half carries those
//! out; this half decides when). It alerts at first sight and again every
//! overdue interval; [`Command::StopRinging`] ends the sound when the
//! priority's ring duration runs out (by default it rings until someone acts
//! or the next repeat). Maximum breaks Do Not Disturb, so it stays an alarm
//! while the server is inhibited.
//!
//! Several devices (spec: Alerts → Alerting on several devices): every
//! device that holds the reminder alerts, including one that only learned of
//! the occurrence from the server, at the occurrence's state now. Before each
//! alert (first, change of style, repeat) the device checks with the server,
//! through a [`ServerCheck`] the platform supplies: the alert waits for the
//! check to come back, for at most the priority's server wait, so an
//! occurrence closed elsewhere that hasn't synced yet is seen first. Priorities
//! with no server wait (Maximum) alert at once and ask for the check anyway.
//! With the server out of reach the alert goes ahead without waiting.
//!
//! Snoozing: a snoozed occurrence is quiet while it goes overdue on schedule
//! and alerts at its current level when the snooze ends. If a known expiry
//! falls inside the snooze, a last-chance alert comes 10 minutes before it
//! (see [`last_chance_at`]): in the priority's due style, never quieter than
//! gentle, titled "Last chance: ..." with "Expires at 23:59". An occurrence
//! snoozed ahead of time opens already snoozed, so it fires quietly.
//!
//! Snooze all and quiet hours: the core holds what they cover as a snooze
//! until their end (`Core::inbox` puts it in `snoozed_until`), with no moment
//! it was set, so the last-chance alert comes through them as it does through
//! any snooze, even when they began after it was due (ADR 0013). The alerter
//! looks again when quiet hours start or end.
//!
//! Pausing: an open occurrence of a paused reminder (or one in a paused list)
//! is not alerted about, and an alert standing for it is closed on the next
//! pass. Its alerts begin again when the pause ends (ADR 0011).
//!
//! Acknowledging quiets the occurrence: while due, until it goes overdue;
//! once overdue, for one overdue interval, after which alerts resume. The
//! time of the acknowledgement is the one recorded on its event.

use std::collections::{HashMap, HashSet};

use crate::core::Core;
use crate::priority::{AlertStyle, Priority, PrioritySettings};
use crate::schedule;
use crate::state::{last_chance_at, DueItem};
use crate::Result;

/// The key of the Done button.
pub const ACTION_DONE: &str = "done";
/// The key of the Skip button.
pub const ACTION_SKIP: &str = "skip";
/// The key of the Snooze button.
pub const ACTION_SNOOZE: &str = "snooze";
/// The key of the Acknowledge button.
pub const ACTION_ACKNOWLEDGE: &str = "acknowledge";
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
    /// The occurrence closed, was snoozed or acknowledged: take its
    /// notification down, and with it the alarm's window and sound.
    Close {
        occurrence_id: String,
    },
    /// An alarm has rung for its priority's ring duration: stop the sound.
    /// The notification and window stay until someone acts or it repeats.
    StopRinging {
        occurrence_id: String,
    },
}

/// How the alerter checks with the server before an alert. The platform
/// half supplies it (on the desktop, the sync loop); the alerter only asks.
///
/// A check is requested with [`request`](Self::request), which returns a
/// ticket, and is [`done`](Self::done) once the device has caught up with the
/// server by a round trip that began after the request. Whatever the server
/// had numbered by then, such as another device's closing, is in the core.
pub trait ServerCheck {
    /// Whether the server can be reached now, or is being tried for the
    /// first time. When it can't, alerts don't wait.
    fn reachable(&self) -> bool;
    /// Asks for a check and returns its ticket.
    fn request(&self) -> u64;
    /// Whether the check with this ticket has come back.
    fn done(&self, ticket: u64) -> bool;
}

/// No server: a standalone device, or one that has not been set up to sync.
/// Alerts never wait.
pub struct NoServer;

impl ServerCheck for NoServer {
    fn reachable(&self) -> bool {
        false
    }
    fn request(&self) -> u64 {
        0
    }
    fn done(&self, _ticket: u64) -> bool {
        true
    }
}

/// An alert held back for a check with the server.
#[derive(Debug, Clone, Copy)]
struct Waiting {
    ticket: u64,
    /// When it alerts anyway: the check was requested plus the server wait.
    until: i64,
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
            AlertStyle::Gentle | AlertStyle::Insistent => {
                (Urgency::Normal, Some(SOUND), GENTLE_TIMEOUT_MS)
            }
            // Critical, never expires, and no themed sound: the app plays
            // its own looping one.
            AlertStyle::Alarm => (Urgency::Critical, None, 0),
        };
        let third = if style == AlertStyle::Alarm {
            (ACTION_ACKNOWLEDGE, "Acknowledge")
        } else {
            (ACTION_SKIP, "Skip")
        };
        Notification {
            occurrence_id: occurrence_id.to_string(),
            title: title.to_string(),
            body: if overdue { "Overdue" } else { "Due" }.to_string(),
            style,
            urgency,
            sound,
            timeout_ms,
            actions: vec![(ACTION_DONE, "Done"), (ACTION_SNOOZE, "Snooze"), third],
        }
    }
}

impl Notification {
    /// "Last chance: Call the plumber · Expires at 23:59".
    fn last_chance(d: &DueItem, style: AlertStyle, expires: &str) -> Self {
        let mut n = Notification::new(&d.occurrence_id, &d.title, style, d.overdue_at <= 0);
        n.title = format!("Last chance: {}", d.title);
        n.body = format!("Expires at {expires}");
        n
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
    /// Whether the sound was already told to stop for the alert standing.
    stopped: bool,
    /// The style last written to the history, so a repeat or a catch-up at
    /// the same style isn't recorded again.
    recorded: Option<AlertStyle>,
    /// An alert that is due but waits for the server's check.
    waiting: Option<Waiting>,
    /// The last-chance alert already given, as when it was due and which
    /// snooze it was in, so each snooze gets one.
    last_chance: Option<(i64, Option<i64>)>,
}

/// The per-device alert planner. It remembers only what it has alerted
/// about since the app started.
pub struct Alerter {
    tracked: HashMap<String, Tracked>,
    settings: fn(Priority) -> PrioritySettings,
}

impl Default for Alerter {
    fn default() -> Self {
        Self::new()
    }
}

impl Alerter {
    pub fn new() -> Self {
        Self::with_settings(Priority::settings)
    }

    /// An alerter that reads each priority's settings from `settings`
    /// instead of the built-ins: how custom priorities (and tests of ring
    /// durations) get in.
    pub fn with_settings(settings: fn(Priority) -> PrioritySettings) -> Self {
        Alerter {
            tracked: HashMap::new(),
            settings,
        }
    }

    /// One pass at `now`. `inhibited` is the notification server's Do Not
    /// Disturb state. Writes the alerts that belong in the history.
    pub fn pass(&mut self, core: &mut Core, now: i64, inhibited: bool) -> Result<Pass> {
        self.pass_with(core, now, inhibited, &NoServer)
    }

    /// [`pass`](Self::pass) on a device that checks with a server before each
    /// alert.
    pub fn pass_with(
        &mut self,
        core: &mut Core,
        now: i64,
        inhibited: bool,
        server: &dyn ServerCheck,
    ) -> Result<Pass> {
        let inbox = core.inbox(now);
        // This device's own limits: its loudest style and "Quiet this
        // device". They only change the style it alerts in; nothing is
        // written as a snooze, and other devices alert in full.
        let limits = core.device_limits(now);
        let open: Vec<_> = inbox.overdue.iter().chain(inbox.due.iter()).collect();
        let mut out = Pass::default();
        let soonest = |t: i64, out: &mut Pass| {
            if t > now {
                out.next_at = Some(out.next_at.map_or(t, |n| n.min(t)));
            }
        };

        // A paused reminder's open occurrence is in neither list: the pause
        // closes the alert standing for it, as closing the occurrence would.
        // The alert comes back when the pause ends, so that is a moment to
        // look again.
        for p in &inbox.paused {
            if let Some(until) = p.pause.until {
                soonest(until, &mut out);
            }
        }

        // "Quiet this device" ending is a moment to look again.
        if let Some(t) = limits.next_change(now) {
            soonest(t, &mut out);
        }

        // Quiet hours starting or ending change what is held: a moment to
        // look again. (A snooze-all's end is a snooze's: see below.)
        if let Some(t) = core.next_quiet_boundary(now) {
            soonest(t, &mut out);
        }
        // So is the end of a snooze-all, even with nothing held by it: the
        // window's chip comes down then.
        for h in core.holding(now) {
            soonest(h.until, &mut out);
        }

        // Occurrences that closed or were paused: their notifications go.
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
            let settings = (self.settings)(d.priority);
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

            // Snoozed or acknowledged: quiet. A snooze that ends alerts
            // again, and so does the quiet period after acknowledging.
            let snoozed = d.snoozed_until.filter(|&u| u > now);
            let acknowledged =
                ack_quiet_until(&settings, d.overdue_at, d.acknowledged_at).filter(|&u| u > now);
            // An acknowledgement with no time on it can't be measured: it
            // quiets until the occurrence closes.
            let unmeasured = d.acknowledged && d.acknowledged_at.is_none();
            if snoozed.is_some() || acknowledged.is_some() || unmeasured {
                tracked.waiting = None;
                // A snooze can't silently turn into a miss: the last-chance
                // alert comes through it. An acknowledgement quiets it too.
                let chance = if acknowledged.is_none() && !unmeasured {
                    snoozed.and_then(|u| last_chance_at(d.expires_at?, d.snoozed_at, u))
                } else {
                    None
                };
                if let Some(at) = chance {
                    let expires_at = d.expires_at.unwrap_or(at);
                    let key = (at, d.snoozed_at);
                    if now < at {
                        soonest(at, &mut out);
                    } else if now < expires_at && tracked.last_chance != Some(key) {
                        let style = settings.due_style.max(AlertStyle::Gentle);
                        let style = if inhibited && !settings.breaks_do_not_disturb {
                            AlertStyle::Silent
                        } else {
                            style
                        };
                        let style = limits.limit(d.priority, style, now);
                        if tracked.recorded != Some(style) {
                            core.record_alert(&d.occurrence_id, style, now)?;
                            tracked.recorded = Some(style);
                        }
                        tracked.last_chance = Some(key);
                        tracked.standing = Some(style);
                        tracked.last_at = now;
                        tracked.stopped = false;
                        let zone =
                            schedule::zone(&core.device_zone()).unwrap_or(jiff::tz::TimeZone::UTC);
                        out.commands.push(Command::Show(Notification::last_chance(
                            d,
                            style,
                            &schedule::clock_time(&zone, expires_at),
                        )));
                    }
                    // The last-chance alert stays up until the occurrence is
                    // acted on or closes.
                    if tracked.last_chance == Some(key) && tracked.standing.is_some() {
                        soonest(snoozed.unwrap_or(now), &mut out);
                        continue;
                    }
                }
                if tracked.standing.take().is_some() {
                    out.commands.push(Command::Close {
                        occurrence_id: d.occurrence_id.clone(),
                    });
                }
                for u in snoozed.into_iter().chain(acknowledged) {
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
            let style = limits.limit(d.priority, style, now);
            let repeats = matches!(style, AlertStyle::Insistent | AlertStyle::Alarm);
            let repeat_at = tracked.last_at + settings.repeat_every();
            let alert = match tracked.standing {
                None => true,
                Some(standing) if standing != style => true,
                Some(_) => repeats && now >= repeat_at,
            };
            // Check with the server first, within the priority's wait.
            let mut alert = alert;
            if alert {
                match tracked.waiting {
                    Some(w) => {
                        if !server.reachable() || server.done(w.ticket) || now >= w.until {
                            tracked.waiting = None;
                        } else {
                            soonest(w.until, &mut out);
                            alert = false;
                        }
                    }
                    None if server.reachable() => {
                        let ticket = server.request();
                        if let Some(wait) = settings.server_wait {
                            tracked.waiting = Some(Waiting {
                                ticket,
                                until: now + wait,
                            });
                            soonest(now + wait, &mut out);
                            alert = false;
                        }
                    }
                    None => {}
                }
            }
            if alert {
                if tracked.recorded != Some(style) {
                    core.record_alert(&d.occurrence_id, style, now)?;
                    tracked.recorded = Some(style);
                }
                tracked.standing = Some(style);
                tracked.last_at = now;
                tracked.stopped = false;
                let mut n =
                    Notification::new(&d.occurrence_id, &d.title, style, d.overdue_at <= now);
                // The note rides along under "Due" or "Overdue".
                if !d.note.is_empty() {
                    n.body = format!("{}\n{}", n.body, d.note);
                }
                out.commands.push(Command::Show(n));
            }
            // An alarm that has rung for its ring duration goes quiet, once.
            if style == AlertStyle::Alarm {
                if let Some(ring) = settings.rings_for() {
                    let stop_at = tracked.last_at + ring;
                    if !tracked.stopped && now >= stop_at {
                        tracked.stopped = true;
                        out.commands.push(Command::StopRinging {
                            occurrence_id: d.occurrence_id.clone(),
                        });
                    } else if !tracked.stopped {
                        soonest(stop_at, &mut out);
                    }
                }
            }
            if repeats {
                soonest(tracked.last_at + settings.repeat_every(), &mut out);
            }
        }
        Ok(out)
    }
}

/// Until when an acknowledgement at `acknowledged_at` quiets an occurrence
/// that goes overdue at `overdue_at`: until it goes overdue if it was
/// acknowledged while due, otherwise for one overdue interval. `None` if it
/// was never acknowledged (or the time is unknown).
fn ack_quiet_until(
    settings: &PrioritySettings,
    overdue_at: i64,
    acknowledged_at: Option<i64>,
) -> Option<i64> {
    let at = acknowledged_at?;
    Some(if at < overdue_at {
        overdue_at
    } else {
        at + settings.repeat_every()
    })
}
