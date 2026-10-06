//! Snooze all and quiet hours (spec: Alerts → Snooze all, Quiet hours;
//! ADR 0013).
//!
//! A snooze-all is a time-boxed hold on every open occurrence in its scope,
//! and on anything that fires before it ends. Quiet hours are the same hold
//! recurring on a schedule. Both are personal settings: events in the
//! personal list, so they apply on all of the user's devices.
//!
//! Neither writes anything per occurrence. A device works out, from the
//! settings alone, whether an occurrence is held at an instant
//! ([`State::hold_on`](crate::State::hold_on)), so devices agree without
//! coordinating, ending a snooze-all early needs one event, and a device that
//! learns of the setting late holds what it should. The snooze each affected
//! occurrence "records" is worked out the same way, for the history
//! ([`State::holds_over`](crate::State::holds_over)).

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp};
use serde::{Deserialize, Serialize};

use crate::condition::{minutes_of, weekday};
use crate::priority::Priority;
use crate::schedule::to_instant;

/// What a snooze-all or quiet hours cover.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "list_id", rename_all = "snake_case")]
pub enum Scope {
    /// Everything the user has.
    All,
    /// One reminder list.
    List(String),
}

impl Scope {
    pub fn covers(&self, list_id: &str) -> bool {
        match self {
            Scope::All => true,
            Scope::List(l) => l == list_id,
        }
    }
}

/// Whether a hold with this setting reaches an occurrence of `priority`:
/// Maximum is left out unless it was included.
fn reaches(include_maximum: bool, priority: Priority) -> bool {
    include_maximum || priority != Priority::Maximum
}

/// A snooze-all: from `from` until `until` (Unix seconds), unless it is ended
/// early.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnoozeAll {
    /// Names it, for ending it early. Made by the device that started it.
    pub id: String,
    pub scope: Scope,
    pub include_maximum: bool,
    /// When it began: what is open then, and anything that fires before
    /// `until`, is held.
    pub from: i64,
    pub until: i64,
}

/// Quiet hours: a snooze-all that recurs, on the `days` it starts, from a
/// time of day until another (which may be the next morning), in the zone of
/// the device that works it out, as floating reminders are.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QuietHours {
    /// The days a stretch of quiet hours starts on, as `MO`..`SU`: "weeknights"
    /// is `MO`..`FR`, and the night from Friday runs into Saturday morning.
    pub days: Vec<String>,
    /// `22:00`.
    pub from: String,
    /// `07:00`. Earlier than `from`: the next morning.
    pub to: String,
    pub scope: Scope,
    pub include_maximum: bool,
}

impl QuietHours {
    /// Whether the quiet hours can be used, or why not.
    pub fn validate(&self) -> Result<(), String> {
        if self.days.is_empty() {
            return Err("choose at least one day".into());
        }
        if let Some(bad) = self.days.iter().find(|d| weekday(d).is_none()) {
            return Err(format!("{bad} is not a day of the week"));
        }
        let (Some(a), Some(b)) = (minutes_of(&self.from), minutes_of(&self.to)) else {
            return Err("quiet hours take times such as 22:00".into());
        };
        if a == b {
            return Err("quiet hours can't start and end at the same time".into());
        }
        Ok(())
    }

    /// The stretch that starts on `date`, as instants.
    fn on(&self, zone: &TimeZone, date: Date) -> Option<(i64, i64)> {
        if !self.days.iter().any(|d| weekday(d) == Some(date.weekday())) {
            return None;
        }
        let (a, b) = (minutes_of(&self.from)?, minutes_of(&self.to)?);
        let start = to_instant(zone, date.at((a / 60) as i8, (a % 60) as i8, 0, 0))?;
        let end_date = if b > a {
            date
        } else {
            date.checked_add(Span::new().days(1)).ok()?
        };
        let end = to_instant(zone, end_date.at((b / 60) as i8, (b % 60) as i8, 0, 0))?;
        (end > start).then_some((start, end))
    }

    /// The stretches that overlap `[from, until)`, earliest first, at most
    /// `max` of them.
    pub fn stretches(&self, zone: &TimeZone, from: i64, until: i64, max: usize) -> Vec<(i64, i64)> {
        let (Ok(a), Ok(b)) = (Timestamp::from_second(from), Timestamp::from_second(until)) else {
            return Vec::new();
        };
        let (first, last) = (zone.to_datetime(a).date(), zone.to_datetime(b).date());
        // One day back: a stretch that started yesterday runs into today.
        let Ok(mut date) = first.checked_sub(Span::new().days(1)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        while date <= last && out.len() < max {
            if let Some((s, e)) = self.on(zone, date) {
                if e > from && s < until {
                    out.push((s, e));
                }
            }
            match date.checked_add(Span::new().days(1)) {
                Ok(d) => date = d,
                Err(_) => break,
            }
        }
        out
    }

    /// The stretch in progress at `t`.
    pub fn stretch_at(&self, zone: &TimeZone, t: i64) -> Option<(i64, i64)> {
        self.stretches(zone, t, t + 1, 4)
            .into_iter()
            .find(|(s, e)| *s <= t && t < *e)
    }

    /// The first start or end of a stretch after `t`.
    pub fn boundary_after(&self, zone: &TimeZone, t: i64) -> Option<i64> {
        // A week and a day on, which holds a stretch on any of the days.
        self.stretches(zone, t, t + 8 * 86_400, 16)
            .into_iter()
            .flat_map(|(s, e)| [s, e])
            .filter(|b| *b > t)
            .min()
    }

    pub fn reaches(&self, list_id: &str, priority: Priority) -> bool {
        self.scope.covers(list_id) && reaches(self.include_maximum, priority)
    }
}

impl SnoozeAll {
    pub fn reaches(&self, list_id: &str, priority: Priority) -> bool {
        self.scope.covers(list_id) && reaches(self.include_maximum, priority)
    }
}

/// What holds an occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    SnoozeAll,
    QuietHours,
}

/// A hold on an occurrence at an instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    /// When the stretch holding it began (or the snooze-all was made).
    pub since: i64,
    /// When it ends: the time alerts resume, unless something else holds.
    pub until: i64,
    pub source: Source,
}

/// How many back-to-back stretches of quiet hours are followed to the end.
const CHAIN: usize = 8;

/// The quiet hours stretch holding an occurrence in `list_id` of `priority`
/// at `t`: the earliest start and the end of the chain of stretches that run
/// into each other (two rules, or the end of one at the start of another).
pub fn quiet_hold(
    rules: &[QuietHours],
    zone: &TimeZone,
    list_id: &str,
    priority: Priority,
    t: i64,
) -> Option<Hold> {
    let mine = || rules.iter().filter(|r| r.reaches(list_id, priority));
    let mut since = i64::MAX;
    let mut until = i64::MIN;
    for r in mine() {
        if let Some((s, e)) = r.stretch_at(zone, t) {
            since = since.min(s);
            until = until.max(e);
        }
    }
    if until == i64::MIN {
        return None;
    }
    for _ in 0..CHAIN {
        let next = mine()
            .filter_map(|r| r.stretch_at(zone, until))
            .map(|(_, e)| e)
            .max()
            .filter(|e| *e > until);
        match next {
            Some(e) => until = e,
            None => break,
        }
    }
    Some(Hold {
        since,
        until,
        source: Source::QuietHours,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone() -> TimeZone {
        TimeZone::UTC
    }

    fn weeknights() -> QuietHours {
        QuietHours {
            days: ["MO", "TU", "WE", "TH", "FR"].map(String::from).to_vec(),
            from: "22:00".into(),
            to: "07:00".into(),
            scope: Scope::All,
            include_maximum: false,
        }
    }

    // 2026-10-05 is a Monday; 00:00 UTC.
    const MONDAY: i64 = 1_791_158_400;
    const HOUR: i64 = 3_600;

    #[test]
    fn a_stretch_crosses_midnight_and_starts_on_its_day() {
        let q = weeknights();
        // Monday night 22:00 to Tuesday 07:00.
        assert_eq!(
            q.stretch_at(&zone(), MONDAY + 23 * HOUR),
            Some((MONDAY + 22 * HOUR, MONDAY + 31 * HOUR))
        );
        // Tuesday 03:00 is still Monday night's.
        assert!(q.stretch_at(&zone(), MONDAY + 27 * HOUR).is_some());
        // Tuesday 07:00 is out, and so is Monday 12:00.
        assert!(q.stretch_at(&zone(), MONDAY + 31 * HOUR).is_none());
        assert!(q.stretch_at(&zone(), MONDAY + 12 * HOUR).is_none());
        // Friday night runs into Saturday morning, but Saturday night has none.
        let friday = MONDAY + 4 * 86_400;
        assert!(q.stretch_at(&zone(), friday + 30 * HOUR).is_some());
        assert!(q
            .stretch_at(&zone(), friday + 24 * HOUR + 23 * HOUR)
            .is_none());
    }

    #[test]
    fn a_stretch_inside_one_day_and_the_next_boundary() {
        let q = QuietHours {
            from: "13:00".into(),
            to: "15:00".into(),
            ..weeknights()
        };
        assert_eq!(q.boundary_after(&zone(), MONDAY), Some(MONDAY + 13 * HOUR));
        assert_eq!(
            q.boundary_after(&zone(), MONDAY + 13 * HOUR),
            Some(MONDAY + 15 * HOUR)
        );
        // After Friday 15:00 the next is Monday 13:00.
        let friday = MONDAY + 4 * 86_400;
        assert_eq!(
            q.boundary_after(&zone(), friday + 15 * HOUR),
            Some(MONDAY + 7 * 86_400 + 13 * HOUR)
        );
    }

    #[test]
    fn bad_quiet_hours_are_refused() {
        let ok = weeknights();
        assert!(ok.validate().is_ok());
        let mut q = ok.clone();
        q.days.clear();
        assert!(q.validate().is_err());
        q = ok.clone();
        q.days = vec!["XX".into()];
        assert!(q.validate().is_err());
        q = ok.clone();
        q.to = "22:00".into();
        assert!(q.validate().is_err());
        q = ok;
        q.from = "25:00".into();
        assert!(q.validate().is_err());
    }

    #[test]
    fn rules_that_run_into_each_other_hold_through_both() {
        let a = weeknights();
        let b = QuietHours {
            from: "07:00".into(),
            to: "08:00".into(),
            days: ["TU", "WE", "TH", "FR", "SA"].map(String::from).to_vec(),
            ..weeknights()
        };
        let h = quiet_hold(&[a, b], &zone(), "l", Priority::High, MONDAY + 23 * HOUR).unwrap();
        assert_eq!((h.since, h.until), (MONDAY + 22 * HOUR, MONDAY + 32 * HOUR));
    }

    #[test]
    fn maximum_is_left_out_unless_included() {
        let mut q = weeknights();
        let t = MONDAY + 23 * HOUR;
        assert!(quiet_hold(std::slice::from_ref(&q), &zone(), "l", Priority::High, t).is_some());
        assert!(quiet_hold(std::slice::from_ref(&q), &zone(), "l", Priority::Maximum, t).is_none());
        q.include_maximum = true;
        assert!(quiet_hold(&[q], &zone(), "l", Priority::Maximum, t).is_some());
    }

    #[test]
    fn a_list_scope_covers_only_that_list() {
        let q = QuietHours {
            scope: Scope::List("home".into()),
            ..weeknights()
        };
        let t = MONDAY + 23 * HOUR;
        assert!(quiet_hold(std::slice::from_ref(&q), &zone(), "home", Priority::Low, t).is_some());
        assert!(quiet_hold(&[q], &zone(), "work", Priority::Low, t).is_none());
    }
}
