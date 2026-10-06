//! Schedules: iCalendar recurrence rules (RRULE) evaluated on the calendar and
//! clock, in a time zone, with the spec's daylight-saving rules.
//!
//! A schedule is a local start (iCalendar's DTSTART, without a zone) and a
//! rule. Instances are generated in local civil time and then turned into
//! instants in a zone. A time that doesn't exist (2:30 on the spring-forward
//! night) fires at the moment the clocks jump (3:00); a time that happens twice
//! (1:30 on the fall-back night) fires once, the first time.
//!
//! Supported: `FREQ` DAILY, WEEKLY, MONTHLY and YEARLY, `INTERVAL`, `COUNT`,
//! `UNTIL`, `BYDAY` (with ordinals for monthly and yearly rules),
//! `BYMONTHDAY` (negative counts from the end), `BYMONTH`, `BYHOUR`,
//! `BYMINUTE`, `BYSECOND` and `WKST`. Hourly and shorter rules are refused:
//! several times a day are several schedules on one reminder.

use jiff::civil::{Date, DateTime, Time, Weekday};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{Span, Timestamp};
use serde::{Deserialize, Serialize};

/// One schedule trigger of a reminder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// The first instance's local date and time, `2026-03-08T02:30:00`. The
    /// rule's missing parts (the time of day, the day of the month, the
    /// weekday) come from here.
    pub start: String,
    /// The recurrence rule, `FREQ=WEEKLY;BYDAY=MO,WE`, with or without a
    /// leading `RRULE:`.
    pub rule: String,
}

/// The common patterns the editor offers. Anything else is a rule written by
/// hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pattern {
    Daily,
    Weekdays,
    /// On chosen days, as `MO`..`SU`.
    Weekly {
        days: Vec<String>,
    },
    /// On a day of the month, 1 to 31, or -1 for the last day.
    MonthlyByDate {
        day: i8,
    },
    /// On the nth weekday of the month: `ordinal` 1 to 4, or -1 for the last.
    MonthlyByWeekday {
        ordinal: i8,
        weekday: String,
    },
}

impl Schedule {
    /// A schedule following `pattern`, at `time` (`09:30`), starting on
    /// `date` (`2026-10-03`).
    pub fn from_pattern(pattern: &Pattern, date: &str, time: &str) -> Result<Schedule, String> {
        let rule = match pattern {
            Pattern::Daily => "FREQ=DAILY".to_string(),
            Pattern::Weekdays => "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_string(),
            Pattern::Weekly { days } => {
                if days.is_empty() {
                    return Err("choose at least one day".into());
                }
                format!("FREQ=WEEKLY;BYDAY={}", days.join(","))
            }
            Pattern::MonthlyByDate { day } => format!("FREQ=MONTHLY;BYMONTHDAY={day}"),
            Pattern::MonthlyByWeekday { ordinal, weekday } => {
                format!("FREQ=MONTHLY;BYDAY={ordinal}{weekday}")
            }
        };
        let time = if time.len() == 5 {
            format!("{time}:00")
        } else {
            time.to_string()
        };
        let schedule = Schedule {
            start: format!("{date}T{time}"),
            rule,
        };
        schedule.validate()?;
        Ok(schedule)
    }

    /// Whether the schedule can be read.
    pub fn validate(&self) -> Result<(), String> {
        Parsed::new(self).map(|_| ())
    }

    /// The instants of this schedule after `after` and up to `until` (both
    /// Unix seconds), earliest first, at most `max` of them, in `zone`.
    /// An unreadable schedule has none.
    pub fn instances(&self, zone: &TimeZone, after: i64, until: i64, max: usize) -> Vec<i64> {
        match Parsed::new(self) {
            Ok(p) => p.instances(zone, after, until, max),
            Err(_) => Vec::new(),
        }
    }
}

/// The zone called `name`, if this device knows it.
pub fn zone(name: &str) -> Option<TimeZone> {
    TimeZone::get(name).ok()
}

/// The name of the zone the system is set to, if it can be told.
pub fn system_zone_name() -> Option<String> {
    TimeZone::system().iana_name().map(str::to_string)
}

/// Unix seconds of the start of the day containing `at`, and of the next day,
/// in `zone`.
pub fn day_bounds(zone: &TimeZone, at: i64) -> (i64, i64) {
    let Ok(ts) = Timestamp::from_second(at) else {
        return (at, at);
    };
    let date = zone.to_datetime(ts).date();
    let start = date
        .to_zoned(zone.clone())
        .map_or(at, |z| z.timestamp().as_second());
    let next = date
        .checked_add(Span::new().days(1))
        .ok()
        .and_then(|d| d.to_zoned(zone.clone()).ok())
        .map_or(at, |z| z.timestamp().as_second());
    (start, next)
}

/// The instant of `hour`:00 on the day after the one `at` falls in, in `zone`.
pub fn tomorrow_at(zone: &TimeZone, at: i64, hour: i8) -> Option<i64> {
    let ts = Timestamp::from_second(at).ok()?;
    let tomorrow = zone
        .to_datetime(ts)
        .date()
        .checked_add(Span::new().days(1))
        .ok()?;
    to_instant(zone, tomorrow.at(hour, 0, 0, 0))
}

/// The time of day `at` is in `zone`, as "23:59".
pub fn clock_time(zone: &TimeZone, at: i64) -> String {
    match Timestamp::from_second(at) {
        Ok(ts) => {
            let t = zone.to_datetime(ts).time();
            format!("{:02}:{:02}", t.hour(), t.minute())
        }
        Err(_) => String::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

struct Parsed {
    start: DateTime,
    freq: Freq,
    interval: i64,
    count: Option<u32>,
    until: Option<DateTime>,
    by_day: Vec<(Option<i8>, Weekday)>,
    by_month_day: Vec<i8>,
    by_month: Vec<i8>,
    by_hour: Vec<i8>,
    by_minute: Vec<i8>,
    by_second: Vec<i8>,
    wkst: Weekday,
}

fn weekday(code: &str) -> Result<Weekday, String> {
    Ok(match code {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        other => return Err(format!("unknown day {other}")),
    })
}

fn numbers(value: &str, lo: i8, hi: i8, what: &str) -> Result<Vec<i8>, String> {
    value
        .split(',')
        .map(|n| {
            n.parse::<i8>()
                .ok()
                .filter(|n| (lo..=hi).contains(n))
                .ok_or_else(|| format!("bad {what} {n}"))
        })
        .collect()
}

/// `20261231`, `20261231T235959` or either with a trailing `Z`.
fn parse_until(value: &str) -> Result<DateTime, String> {
    let v = value.trim_end_matches('Z');
    let bad = || format!("bad UNTIL {value}");
    let num = |s: &str| s.parse::<i16>().map_err(|_| bad());
    if v.len() < 8 || !v.is_ascii() {
        return Err(bad());
    }
    let (y, m, d) = (num(&v[0..4])?, num(&v[4..6])?, num(&v[6..8])?);
    let (h, mi, s) = if v.len() >= 15 && &v[8..9] == "T" {
        (num(&v[9..11])?, num(&v[11..13])?, num(&v[13..15])?)
    } else if v.len() == 8 {
        (23, 59, 59)
    } else {
        return Err(bad());
    };
    DateTime::new(y, m as i8, d as i8, h as i8, mi as i8, s as i8, 0).map_err(|_| bad())
}

impl Parsed {
    fn new(s: &Schedule) -> Result<Parsed, String> {
        let start: DateTime = s
            .start
            .parse()
            .map_err(|_| format!("bad start {}", s.start))?;
        let rule = s.rule.trim();
        let rule = rule.strip_prefix("RRULE:").unwrap_or(rule);
        let mut p = Parsed {
            start,
            freq: Freq::Daily,
            interval: 1,
            count: None,
            until: None,
            by_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month: Vec::new(),
            by_hour: Vec::new(),
            by_minute: Vec::new(),
            by_second: Vec::new(),
            wkst: Weekday::Monday,
        };
        let mut freq = None;
        for part in rule.split(';').filter(|p| !p.is_empty()) {
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| format!("bad rule part {part}"))?;
            match key.to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match value.to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        "HOURLY" | "MINUTELY" | "SECONDLY" => {
                            return Err("hourly and shorter rules aren't supported: use several \
                                 schedules for several times a day"
                                .into())
                        }
                        other => return Err(format!("unknown FREQ {other}")),
                    })
                }
                "INTERVAL" => {
                    p.interval = value
                        .parse::<i64>()
                        .ok()
                        .filter(|n| (1..=10_000).contains(n))
                        .ok_or("bad INTERVAL")?
                }
                "COUNT" => {
                    p.count = Some(
                        value
                            .parse::<u32>()
                            .ok()
                            .filter(|n| *n > 0)
                            .ok_or("bad COUNT")?,
                    )
                }
                "UNTIL" => p.until = Some(parse_until(value)?),
                "BYDAY" => {
                    for d in value.split(',') {
                        let d = d.to_ascii_uppercase();
                        if d.len() < 2 || !d.is_ascii() {
                            return Err(format!("bad day {d}"));
                        }
                        let (ord, code) = d.split_at(d.len() - 2);
                        let ord = if ord.is_empty() {
                            None
                        } else {
                            Some(
                                ord.parse::<i8>()
                                    .ok()
                                    .filter(|n| *n != 0 && (-53..=53).contains(n))
                                    .ok_or_else(|| format!("bad day {d}"))?,
                            )
                        };
                        p.by_day.push((ord, weekday(code)?));
                    }
                }
                "BYMONTHDAY" => {
                    p.by_month_day = numbers(value, -31, 31, "day of the month")?;
                    if p.by_month_day.contains(&0) {
                        return Err("bad day of the month 0".into());
                    }
                }
                "BYMONTH" => p.by_month = numbers(value, 1, 12, "month")?,
                "BYHOUR" => p.by_hour = numbers(value, 0, 23, "hour")?,
                "BYMINUTE" => p.by_minute = numbers(value, 0, 59, "minute")?,
                "BYSECOND" => p.by_second = numbers(value, 0, 59, "second")?,
                "WKST" => p.wkst = weekday(&value.to_ascii_uppercase())?,
                other => return Err(format!("{other} isn't supported")),
            }
        }
        p.freq = freq.ok_or("the rule has no FREQ")?;
        if p.count.is_some() && p.until.is_some() {
            return Err("COUNT and UNTIL can't both be set".into());
        }
        Ok(p)
    }

    /// The times of day each matching date fires at, earliest first.
    fn times(&self) -> Vec<Time> {
        let pick = |v: &[i8], d: i8| if v.is_empty() { vec![d] } else { v.to_vec() };
        let mut out = Vec::new();
        for h in pick(&self.by_hour, self.start.hour()) {
            for m in pick(&self.by_minute, self.start.minute()) {
                for s in pick(&self.by_second, self.start.second()) {
                    if let Ok(t) = Time::new(h, m, s, 0) {
                        out.push(t);
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The days of a month the rule picks, earliest first.
    fn days_of_month(&self, year: i16, month: i8) -> Vec<Date> {
        let Ok(first) = Date::new(year, month, 1) else {
            return Vec::new();
        };
        let dim = first.days_in_month();
        let mut by_date: Vec<i8> = self
            .by_month_day
            .iter()
            .map(|d| if *d < 0 { dim + 1 + *d } else { *d })
            .filter(|d| (1..=dim).contains(d))
            .collect();
        let by_day: Vec<i8> = if self.by_day.is_empty() {
            Vec::new()
        } else {
            let mut v = Vec::new();
            for day in 1..=dim {
                let Ok(date) = Date::new(year, month, day) else {
                    continue;
                };
                let wd = date.weekday();
                let from_end = (dim - day) / 7 + 1;
                let from_start = (day - 1) / 7 + 1;
                let hit = self.by_day.iter().any(|(ord, w)| {
                    *w == wd
                        && match ord {
                            None => true,
                            Some(n) if *n > 0 => *n == from_start,
                            Some(n) => -*n == from_end,
                        }
                });
                if hit {
                    v.push(day);
                }
            }
            v
        };
        let days: Vec<i8> = match (self.by_month_day.is_empty(), self.by_day.is_empty()) {
            (true, true) => {
                if self.start.day() <= dim {
                    vec![self.start.day()]
                } else {
                    Vec::new()
                }
            }
            (false, true) => {
                by_date.sort();
                by_date.dedup();
                by_date
            }
            (true, false) => by_day,
            (false, false) => by_date.into_iter().filter(|d| by_day.contains(d)).collect(),
        };
        days.into_iter()
            .filter_map(|d| Date::new(year, month, d).ok())
            .collect()
    }

    fn month_ok(&self, date: Date) -> bool {
        self.by_month.is_empty() || self.by_month.contains(&date.month())
    }

    /// The dates of period `k`, earliest first. Most periods have some; a
    /// period can have none (the 31st in a short month).
    fn dates_in_period(&self, k: i64) -> Option<Vec<Date>> {
        let n = self.interval.checked_mul(k)?;
        let sd = self.start.date();
        match self.freq {
            Freq::Daily => {
                let d = sd.checked_add(Span::new().try_days(n).ok()?).ok()?;
                let weekday_ok =
                    self.by_day.is_empty() || self.by_day.iter().any(|(_, w)| *w == d.weekday());
                let day_ok = self.by_month_day.is_empty()
                    || self.by_month_day.iter().any(|m| {
                        let dim = d.days_in_month();
                        let m = if *m < 0 { dim + 1 + *m } else { *m };
                        m == d.day()
                    });
                Some(if weekday_ok && day_ok && self.month_ok(d) {
                    vec![d]
                } else {
                    Vec::new()
                })
            }
            Freq::Weekly => {
                let back = (i64::from(sd.weekday().to_sunday_zero_offset())
                    - i64::from(self.wkst.to_sunday_zero_offset()))
                .rem_euclid(7);
                let week = sd
                    .checked_add(Span::new().try_days(n.checked_mul(7)? - back).ok()?)
                    .ok()?;
                let mut v = Vec::new();
                for i in 0..7 {
                    let d = week.checked_add(Span::new().days(i)).ok()?;
                    let hit = if self.by_day.is_empty() {
                        d.weekday() == sd.weekday()
                    } else {
                        self.by_day.iter().any(|(_, w)| *w == d.weekday())
                    };
                    if hit && self.month_ok(d) {
                        v.push(d);
                    }
                }
                Some(v)
            }
            Freq::Monthly => {
                let months = i64::from(sd.year()) * 12 + i64::from(sd.month() - 1) + n;
                let (y, m) = (months.div_euclid(12), months.rem_euclid(12) + 1);
                let y = i16::try_from(y).ok()?;
                let month = i8::try_from(m).ok()?;
                if !self.by_month.is_empty() && !self.by_month.contains(&month) {
                    return Some(Vec::new());
                }
                Some(self.days_of_month(y, month))
            }
            Freq::Yearly => {
                let y = i16::try_from(i64::from(sd.year()) + n).ok()?;
                let months = if self.by_month.is_empty() {
                    vec![sd.month()]
                } else {
                    let mut m = self.by_month.clone();
                    m.sort();
                    m.dedup();
                    m
                };
                Some(
                    months
                        .into_iter()
                        .flat_map(|m| self.days_of_month(y, m))
                        .collect(),
                )
            }
        }
    }

    /// A period to start from that is sure to come before `local`.
    fn first_period(&self, local: Date) -> i64 {
        if self.count.is_some() {
            return 0; // counting needs every instance from the start
        }
        let sd = self.start.date();
        let days = i64::from(sd.until(local).map_or(0, |s| s.get_days()));
        let months = (i64::from(local.year()) - i64::from(sd.year())) * 12
            + i64::from(local.month())
            - i64::from(sd.month());
        let periods = match self.freq {
            Freq::Daily => days / self.interval,
            Freq::Weekly => days / (7 * self.interval),
            Freq::Monthly => months / self.interval,
            Freq::Yearly => months / (12 * self.interval),
        };
        (periods - 2).max(0)
    }

    fn instances(&self, zone: &TimeZone, after: i64, until: i64, max: usize) -> Vec<i64> {
        let mut out = Vec::new();
        let (Ok(after_ts), Ok(until_ts)) =
            (Timestamp::from_second(after), Timestamp::from_second(until))
        else {
            return out;
        };
        // Local times and instants differ by at most a day or so.
        let day = Span::new().days(2);
        let Ok(after_local) = zone.to_datetime(after_ts).checked_sub(day) else {
            return out;
        };
        let until_local = zone
            .to_datetime(until_ts)
            .checked_add(day)
            .ok()
            .into_iter()
            .chain(self.until)
            .min();
        let times = self.times();
        let mut k = self.first_period(after_local.date());
        let mut seen: u32 = 0;
        let mut empty = 0;
        while let Some(dates) = self.dates_in_period(k) {
            k += 1;
            if dates.is_empty() {
                empty += 1;
                if empty > 2_000 {
                    break;
                }
                continue;
            }
            empty = 0;
            for date in dates {
                for time in &times {
                    let dt = date.to_datetime(*time);
                    if dt < self.start {
                        continue;
                    }
                    if let Some(limit) = self.count {
                        if seen >= limit {
                            return out;
                        }
                        seen += 1;
                    }
                    if until_local.is_some_and(|u| dt > u) {
                        return out;
                    }
                    if dt < after_local {
                        continue;
                    }
                    let Some(at) = to_instant(zone, dt) else {
                        continue;
                    };
                    if at > after && at <= until {
                        out.push(at);
                        if out.len() >= max {
                            return out;
                        }
                    }
                }
            }
        }
        out
    }
}

/// The instant a local time means in `zone`: the first of two, and for a time
/// that doesn't exist the moment the clocks jump.
pub(crate) fn to_instant(zone: &TimeZone, dt: DateTime) -> Option<i64> {
    let ambiguous = zone.to_ambiguous_timestamp(dt);
    match ambiguous.offset() {
        AmbiguousOffset::Unambiguous { .. } | AmbiguousOffset::Fold { .. } => {
            ambiguous.earlier().ok().map(|t| t.as_second())
        }
        AmbiguousOffset::Gap { .. } => {
            let first = ambiguous.earlier().ok()?;
            let last = ambiguous.later().ok()?;
            // The transition lies between the two readings of the time.
            let (lo, hi) = if first < last {
                (first, last)
            } else {
                (last, first)
            };
            let transition = zone
                .following(lo)
                .next()
                .map(|t| t.timestamp())
                .filter(|t| *t > lo && *t <= hi);
            Some(transition.unwrap_or(hi).as_second())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(zone: &TimeZone, s: &str) -> i64 {
        let dt: DateTime = s.parse().unwrap();
        to_instant(zone, dt).unwrap()
    }

    fn local(zone: &TimeZone, at: i64) -> String {
        zone.to_datetime(Timestamp::from_second(at).unwrap())
            .to_string()
    }

    fn sched(start: &str, rule: &str) -> Schedule {
        Schedule {
            start: start.into(),
            rule: rule.into(),
        }
    }

    fn all(s: &Schedule, zone: &TimeZone, from: &str, to: &str, max: usize) -> Vec<String> {
        s.instances(zone, ts(zone, from), ts(zone, to), max)
            .into_iter()
            .map(|t| local(zone, t))
            .collect()
    }

    #[test]
    fn daily_weekdays_and_weekly() {
        let utc = TimeZone::UTC;
        let daily = sched("2026-10-01T07:00:00", "FREQ=DAILY");
        assert_eq!(
            all(
                &daily,
                &utc,
                "2026-10-02T00:00:00",
                "2026-10-04T23:00:00",
                10
            ),
            [
                "2026-10-02T07:00:00",
                "2026-10-03T07:00:00",
                "2026-10-04T07:00:00"
            ]
        );
        let weekdays = Schedule::from_pattern(&Pattern::Weekdays, "2026-10-01", "07:00").unwrap();
        // 2026-10-03 is a Saturday.
        assert_eq!(
            all(
                &weekdays,
                &utc,
                "2026-10-02T08:00:00",
                "2026-10-06T23:00:00",
                10
            ),
            ["2026-10-05T07:00:00", "2026-10-06T07:00:00"]
        );
        let weekly = sched("2026-10-01T18:30:00", "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH");
        assert_eq!(
            all(
                &weekly,
                &utc,
                "2026-10-01T00:00:00",
                "2026-10-31T00:00:00",
                10
            ),
            [
                "2026-10-01T18:30:00",
                "2026-10-12T18:30:00",
                "2026-10-15T18:30:00",
                "2026-10-26T18:30:00",
                "2026-10-29T18:30:00"
            ]
        );
    }

    #[test]
    fn monthly_by_date_and_by_weekday() {
        let utc = TimeZone::UTC;
        let by_date =
            Schedule::from_pattern(&Pattern::MonthlyByDate { day: 31 }, "2026-01-31", "09:00")
                .unwrap();
        // Months without a 31st are skipped.
        assert_eq!(
            all(
                &by_date,
                &utc,
                "2026-01-01T00:00:00",
                "2026-05-01T00:00:00",
                10
            ),
            ["2026-01-31T09:00:00", "2026-03-31T09:00:00"]
        );
        let last_day = sched("2026-01-01T09:00:00", "FREQ=MONTHLY;BYMONTHDAY=-1");
        assert_eq!(
            all(
                &last_day,
                &utc,
                "2026-01-01T00:00:00",
                "2026-03-01T00:00:00",
                10
            ),
            ["2026-01-31T09:00:00", "2026-02-28T09:00:00"]
        );
        // The first Monday of the month, and the last Friday.
        let first_monday = Schedule::from_pattern(
            &Pattern::MonthlyByWeekday {
                ordinal: 1,
                weekday: "MO".into(),
            },
            "2026-10-01",
            "08:00",
        )
        .unwrap();
        assert_eq!(
            all(
                &first_monday,
                &utc,
                "2026-10-01T00:00:00",
                "2026-12-31T00:00:00",
                10
            ),
            [
                "2026-10-05T08:00:00",
                "2026-11-02T08:00:00",
                "2026-12-07T08:00:00"
            ]
        );
        let last_friday = sched("2026-10-01T08:00:00", "FREQ=MONTHLY;BYDAY=-1FR");
        assert_eq!(
            all(
                &last_friday,
                &utc,
                "2026-10-01T00:00:00",
                "2026-11-30T00:00:00",
                10
            ),
            ["2026-10-30T08:00:00", "2026-11-27T08:00:00"]
        );
    }

    #[test]
    fn count_until_yearly_and_times_of_day() {
        let utc = TimeZone::UTC;
        let counted = sched("2026-10-01T07:00:00", "FREQ=DAILY;COUNT=3");
        assert_eq!(
            all(
                &counted,
                &utc,
                "2026-09-01T00:00:00",
                "2027-01-01T00:00:00",
                10
            )
            .len(),
            3
        );
        // Counting starts at the start, not at the range.
        assert_eq!(
            all(
                &counted,
                &utc,
                "2026-10-02T12:00:00",
                "2027-01-01T00:00:00",
                10
            ),
            ["2026-10-03T07:00:00"]
        );
        let until = sched("2026-10-01T07:00:00", "FREQ=DAILY;UNTIL=20261002");
        assert_eq!(
            all(
                &until,
                &utc,
                "2026-09-01T00:00:00",
                "2027-01-01T00:00:00",
                10
            )
            .len(),
            2
        );
        let yearly = sched("2024-02-29T10:00:00", "FREQ=YEARLY");
        assert_eq!(
            all(
                &yearly,
                &utc,
                "2024-01-01T00:00:00",
                "2033-01-01T00:00:00",
                10
            ),
            [
                "2024-02-29T10:00:00",
                "2028-02-29T10:00:00",
                "2032-02-29T10:00:00"
            ]
        );
        let twice = sched("2026-10-01T08:00:00", "FREQ=DAILY;BYHOUR=8,20");
        assert_eq!(
            all(
                &twice,
                &utc,
                "2026-10-01T00:00:00",
                "2026-10-01T23:00:00",
                10
            ),
            ["2026-10-01T08:00:00", "2026-10-01T20:00:00"]
        );
    }

    #[test]
    fn a_range_far_from_the_start_is_found_directly() {
        let utc = TimeZone::UTC;
        let s = sched("2000-01-01T07:00:00", "FREQ=WEEKLY;BYDAY=SA");
        assert_eq!(
            all(&s, &utc, "2026-10-01T00:00:00", "2026-10-15T00:00:00", 10),
            ["2026-10-03T07:00:00", "2026-10-10T07:00:00"]
        );
    }

    #[test]
    fn bad_rules_are_refused() {
        for rule in [
            "FREQ=HOURLY",
            "FREQ=NEVER",
            "BYDAY=MO",
            "FREQ=DAILY;COUNT=2;UNTIL=20270101",
            "FREQ=DAILY;INTERVAL=0",
            "FREQ=WEEKLY;BYDAY=XX",
            "FREQ=DAILY;BYSETPOS=1",
            "nonsense",
        ] {
            assert!(
                sched("2026-10-01T07:00:00", rule).validate().is_err(),
                "{rule}"
            );
        }
        assert!(sched("tomorrow", "FREQ=DAILY").validate().is_err());
        assert!(sched("2026-10-01T07:00:00", "RRULE:FREQ=DAILY")
            .validate()
            .is_ok());
        assert!(
            Schedule::from_pattern(&Pattern::Weekly { days: vec![] }, "2026-10-01", "07:00")
                .is_err()
        );
    }

    #[test]
    fn the_spring_forward_night_fires_a_missing_time_at_the_jump() {
        let ny = zone("America/New_York").unwrap();
        let s = sched("2026-03-07T02:30:00", "FREQ=DAILY");
        let got = s.instances(
            &ny,
            ts(&ny, "2026-03-07T12:00:00"),
            ts(&ny, "2026-03-10T00:00:00"),
            10,
        );
        // 2:30 doesn't exist on 2026-03-08: the clocks jump from 2:00 to 3:00.
        assert_eq!(local(&ny, got[0]), "2026-03-08T03:00:00");
        assert_eq!(got[0], 1_772_953_200); // 2026-03-08T07:00:00Z
        assert_eq!(local(&ny, got[1]), "2026-03-09T02:30:00");
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn the_fall_back_night_fires_once_the_first_time() {
        let ny = zone("America/New_York").unwrap();
        let s = sched("2026-10-31T01:30:00", "FREQ=DAILY");
        let got = s.instances(
            &ny,
            ts(&ny, "2026-10-31T12:00:00"),
            ts(&ny, "2026-11-03T00:00:00"),
            10,
        );
        assert_eq!(got.len(), 2);
        // 1:30 happens twice on 2026-11-01; the first is still on daylight time.
        assert_eq!(got[0], 1_793_511_000); // 2026-11-01T05:30:00Z
        assert_eq!(got[1] - got[0], 25 * 3600);
    }
}
