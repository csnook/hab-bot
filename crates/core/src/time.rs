//! Schedules: iCalendar recurrence rules expanded as wall-clock times, then placed on
//! the timeline in a time zone with the spec's daylight-saving rules.
//!
//! The `rrule` crate does the expansion with the wall-clock time passed off as UTC, so it
//! never applies a zone's offsets itself. [`resolve`] does that.

use crate::{Error, Millis, Result};
use chrono::{DateTime, Duration, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

const WALL_FORMAT: &str = "%Y-%m-%dT%H:%M";
/// A cap on how many instances one expansion returns, against runaway rules.
const LIMIT: u16 = 1000;

/// Parses a wall-clock time as the UI and the events write it: `2026-10-03T07:00`.
pub fn parse_wall(s: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, WALL_FORMAT)
        .map_err(|_| Error::BadSchedule(format!("bad time {s:?}")))
}

pub fn format_wall(w: NaiveDateTime) -> String {
    w.format(WALL_FORMAT).to_string()
}

pub fn parse_zone(name: &str) -> Result<Tz> {
    name.parse::<Tz>()
        .map_err(|_| Error::BadSchedule(format!("unknown time zone {name:?}")))
}

/// The wall-clock time on the clock of `tz` at the instant `at`.
pub fn wall_at(at: Millis, tz: Tz) -> NaiveDateTime {
    DateTime::<Utc>::from_timestamp_millis(at)
        .unwrap_or_default()
        .with_timezone(&tz)
        .naive_local()
}

/// Puts a wall-clock time on the timeline.
/// - A time that doesn't exist (2:30 on the spring-forward night) is the instant the clock
///   jumps to, so 3:00.
/// - A time that happens twice (1:30 on the fall-back night) is the first one.
pub fn resolve(wall: NaiveDateTime, tz: Tz) -> Millis {
    match tz.from_local_datetime(&wall) {
        LocalResult::Single(t) => t.timestamp_millis(),
        LocalResult::Ambiguous(first, _) => first.timestamp_millis(),
        LocalResult::None => {
            // In a gap. Find the first instant after the gap by bisecting for where the
            // offset changes, searching up to a day either side.
            let guess = wall.and_utc();
            let mut lo = guess - Duration::hours(24);
            let mut hi = guess + Duration::hours(24);
            let before = tz.offset_from_utc_datetime(&lo.naive_utc());
            while hi - lo > Duration::seconds(1) {
                let mid = lo + (hi - lo) / 2;
                if tz.offset_from_utc_datetime(&mid.naive_utc()) == before {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            hi.timestamp_millis()
        }
    }
}

/// Checks a rule is understood, so a bad one is refused when the reminder is made.
pub fn validate(rule: &str, start: NaiveDateTime) -> Result<()> {
    set(rule, start).map(|_| ())
}

fn set(rule: &str, start: NaiveDateTime) -> Result<rrule::RRuleSet> {
    let text = format!(
        "DTSTART:{}\nRRULE:{}",
        start.format("%Y%m%dT%H%M%SZ"),
        rule.trim()
    );
    text.parse()
        .map_err(|e| Error::BadSchedule(format!("{rule:?}: {e}")))
}

/// The instances of `rule` whose wall-clock time lies in `[from, to]`, in order.
pub fn instances(
    rule: &str,
    start: NaiveDateTime,
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> Result<Vec<NaiveDateTime>> {
    let utc = rrule::Tz::UTC;
    let set = set(rule, start)?
        .after(utc.from_utc_datetime(&(from - Duration::seconds(1))))
        .before(utc.from_utc_datetime(&(to + Duration::seconds(1))));
    Ok(set
        .all(LIMIT)
        .dates
        .into_iter()
        .map(|d| d.naive_utc())
        .collect())
}
