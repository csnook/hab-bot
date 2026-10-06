//! Countdowns: a trigger that occurs a set time after the reminder's last
//! occurrence was closed.
//!
//! Minutes and hours count elapsed time, whatever the time zone. Days and
//! weeks are calendar days in the reminder's zone: they land on the same
//! local time of day as the closing, or on a chosen time of day ("3 days
//! later, at 9:00"). A time that doesn't exist (the spring-forward night)
//! falls at the moment the clocks jump, and one that happens twice at the
//! first.

use jiff::civil::Time;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp};
use serde::{Deserialize, Serialize};

use crate::schedule::to_instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountdownUnit {
    Minutes,
    Hours,
    Days,
    Weeks,
}

/// How long after an occurrence closes the next one comes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Countdown {
    pub amount: u32,
    pub unit: CountdownUnit,
    /// The time of day (`09:00`) to fire at, for days and weeks. Without it a
    /// countdown of days or weeks fires at the time of day it was closed.
    pub at: Option<String>,
}

impl Countdown {
    /// A countdown of hours or less counts elapsed time and has no time zone.
    pub fn is_elapsed(&self) -> bool {
        matches!(self.unit, CountdownUnit::Minutes | CountdownUnit::Hours)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.amount == 0 || self.amount > 100_000 {
            return Err("the countdown needs an amount of at least 1".into());
        }
        match (&self.at, self.is_elapsed()) {
            (Some(_), true) => Err("only a countdown of days or longer has a time of day".into()),
            (Some(at), false) => parse_time(at)
                .map(|_| ())
                .ok_or_else(|| format!("bad time of day {at}")),
            (None, _) => Ok(()),
        }
    }

    /// When the countdown ends if the last occurrence closed at `closed_at`
    /// (Unix seconds), in `zone`. Elapsed countdowns ignore the zone.
    pub fn next_after(&self, zone: &TimeZone, closed_at: i64) -> Option<i64> {
        let amount = i64::from(self.amount);
        match self.unit {
            CountdownUnit::Minutes => closed_at.checked_add(amount.checked_mul(60)?),
            CountdownUnit::Hours => closed_at.checked_add(amount.checked_mul(3600)?),
            CountdownUnit::Days | CountdownUnit::Weeks => {
                let days = if self.unit == CountdownUnit::Weeks {
                    amount.checked_mul(7)?
                } else {
                    amount
                };
                let local = zone.to_datetime(Timestamp::from_second(closed_at).ok()?);
                let time = match &self.at {
                    Some(at) => parse_time(at)?,
                    None => local.time(),
                };
                let date = local
                    .date()
                    .checked_add(Span::new().try_days(days).ok()?)
                    .ok()?;
                to_instant(zone, date.to_datetime(time))
            }
        }
    }
}

fn parse_time(at: &str) -> Option<Time> {
    let full = if at.len() == 5 {
        format!("{at}:00")
    } else {
        at.to_string()
    };
    full.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::DateTime;

    fn at(zone: &TimeZone, s: &str) -> i64 {
        to_instant(zone, s.parse::<DateTime>().unwrap()).unwrap()
    }

    fn local(zone: &TimeZone, t: i64) -> String {
        zone.to_datetime(Timestamp::from_second(t).unwrap())
            .to_string()
    }

    fn c(amount: u32, unit: CountdownUnit, at: Option<&str>) -> Countdown {
        Countdown {
            amount,
            unit,
            at: at.map(str::to_string),
        }
    }

    #[test]
    fn hours_are_elapsed_time_in_any_zone() {
        let ny = TimeZone::get("America/New_York").unwrap();
        let tokyo = TimeZone::get("Asia/Tokyo").unwrap();
        // Across the spring-forward gap eight hours is still eight hours.
        let t = at(&ny, "2026-03-08T00:30:00");
        let next = c(8, CountdownUnit::Hours, None);
        assert_eq!(next.next_after(&ny, t), Some(t + 8 * 3600));
        assert_eq!(next.next_after(&tokyo, t), Some(t + 8 * 3600));
        assert_eq!(
            c(90, CountdownUnit::Minutes, None).next_after(&ny, t),
            Some(t + 5400)
        );
    }

    #[test]
    fn days_are_calendar_days_at_a_time_of_day_or_the_closing_time() {
        let ny = TimeZone::get("America/New_York").unwrap();
        let closed = at(&ny, "2026-10-01T14:20:00");
        let nine = c(3, CountdownUnit::Days, Some("09:00"));
        assert_eq!(
            local(&ny, nine.next_after(&ny, closed).unwrap()),
            "2026-10-04T09:00:00"
        );
        let same = c(3, CountdownUnit::Days, None);
        assert_eq!(
            local(&ny, same.next_after(&ny, closed).unwrap()),
            "2026-10-04T14:20:00"
        );
        let weeks = c(2, CountdownUnit::Weeks, Some("07:30"));
        assert_eq!(
            local(&ny, weeks.next_after(&ny, closed).unwrap()),
            "2026-10-15T07:30:00"
        );
        // The time of day follows the zone it is read in.
        let tokyo = TimeZone::get("Asia/Tokyo").unwrap();
        assert_ne!(
            nine.next_after(&ny, closed),
            nine.next_after(&tokyo, closed)
        );
    }

    #[test]
    fn calendar_days_follow_daylight_saving() {
        let ny = TimeZone::get("America/New_York").unwrap();
        // Spring forward 2026-03-08: a day later at 9:00 is 23 hours on.
        let closed = at(&ny, "2026-03-07T09:00:00");
        let d = c(1, CountdownUnit::Days, Some("09:00"));
        assert_eq!(d.next_after(&ny, closed), Some(closed + 23 * 3600));
        // 2:30 doesn't exist that night: it falls at 3:00.
        let gap = c(1, CountdownUnit::Days, Some("02:30"));
        let before = at(&ny, "2026-03-07T12:00:00");
        assert_eq!(
            local(&ny, gap.next_after(&ny, before).unwrap()),
            "2026-03-08T03:00:00"
        );
        // 1:30 happens twice on 2026-11-01: the first time.
        let fold = c(1, CountdownUnit::Days, Some("01:30"));
        let before = at(&ny, "2026-10-31T12:00:00");
        let t = fold.next_after(&ny, before).unwrap();
        // EDT (UTC-4) is the first reading of 1:30, so it is 05:30 UTC.
        assert_eq!(t, at(&TimeZone::UTC, "2026-11-01T05:30:00"));
    }

    #[test]
    fn validation() {
        assert!(c(0, CountdownUnit::Days, None).validate().is_err());
        assert!(c(2, CountdownUnit::Hours, Some("09:00"))
            .validate()
            .is_err());
        assert!(c(2, CountdownUnit::Days, Some("25:00")).validate().is_err());
        assert!(c(2, CountdownUnit::Days, Some("9:00")).validate().is_err());
        assert!(c(2, CountdownUnit::Days, Some("09:00")).validate().is_ok());
        assert!(c(8, CountdownUnit::Hours, None).validate().is_ok());
    }
}
