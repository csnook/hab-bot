//! Which instants a reminder's triggers produce.

use crate::state::{CountdownUnit, Reminder, State, Trigger};
use crate::time::{self, parse_wall, parse_zone};
use crate::Millis;
use chrono::Duration;
use chrono_tz::Tz;

/// One instant a reminder is scheduled to come due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub scheduled_at: Millis,
    /// Its identity within the reminder. Pinned reminders use the instant; floating ones
    /// the wall-clock time, so devices in different zones agree on which instance it is.
    pub key: String,
}

/// The reminder's instances with `after < scheduled_at <= to`, in order.
/// `device` is the zone floating reminders follow.
pub fn instances(
    state: &State,
    reminder: &Reminder,
    device: Tz,
    after: Millis,
    to: Millis,
) -> Vec<Instance> {
    let pinned = reminder.tz.as_deref().and_then(|z| parse_zone(z).ok());
    let tz = pinned.unwrap_or(device);
    let mut found: Vec<Instance> = Vec::new();
    for trigger in &reminder.triggers {
        match trigger {
            Trigger::OneOff { at } => {
                if *at > after && *at <= to {
                    found.push(Instance {
                        scheduled_at: *at,
                        key: at.to_string(),
                    });
                }
            }
            Trigger::Countdown {
                unit,
                amount,
                at,
                last_done,
            } => {
                // A countdown restarts only once its occurrence has closed.
                if state.open_occurrence(&reminder.id).is_some() {
                    continue;
                }
                let base = state.last_closed(&reminder.id).or(*last_done);
                let next = match base {
                    Some(base) => countdown_next(base, *unit, *amount, at.as_deref(), tz),
                    // "Never done": it fires at once.
                    None => reminder.created_at,
                };
                if next > after && next <= to {
                    found.push(Instance {
                        scheduled_at: next,
                        key: next.to_string(),
                    });
                }
            }
            Trigger::Schedule { rule, start } => {
                let Ok(start) = parse_wall(start) else {
                    continue;
                };
                // A day of slack either side, since offsets differ from the wall clock's.
                let from_wall = time::wall_at(after, tz) - Duration::days(1);
                let to_wall = time::wall_at(to, tz) + Duration::days(1);
                let Ok(walls) = time::instances(rule, start, from_wall, to_wall) else {
                    continue;
                };
                for wall in walls {
                    let at = time::resolve(wall, tz);
                    if at > after && at <= to {
                        let key = if pinned.is_some() {
                            at.to_string()
                        } else {
                            time::format_wall(wall)
                        };
                        found.push(Instance {
                            scheduled_at: at,
                            key,
                        });
                    }
                }
            }
        }
    }
    found.sort_by_key(|i| i.scheduled_at);
    found.dedup_by_key(|i| i.scheduled_at);
    found
}

/// When a countdown that restarted at `base` next fires.
fn countdown_next(
    base: Millis,
    unit: CountdownUnit,
    amount: u32,
    at: Option<&str>,
    tz: Tz,
) -> Millis {
    match unit {
        CountdownUnit::Hours => base + amount as Millis * 3_600_000,
        CountdownUnit::Days => {
            let date = time::wall_at(base, tz).date() + Duration::days(amount as i64);
            let time = at
                .and_then(|a| chrono::NaiveTime::parse_from_str(a, "%H:%M").ok())
                // Without a time of day, the same time of day as the restart.
                .unwrap_or_else(|| time::wall_at(base, tz).time());
            time::resolve(date.and_time(time), tz)
        }
    }
}
