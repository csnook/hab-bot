//! Time-based conditions (spec: Sources → Time → Time-based conditions):
//! days of the week, a time window that can cross midnight, a date range or
//! a season, and daylight or darkness.
//!
//! A condition is evaluated at the instant a trigger would fire, in the
//! reminder's time zone, by [`Condition::holds`], a pure function of that
//! instant, the zone and the home location. Two devices holding the same
//! reminder and in the same zone, with the same home, therefore agree on
//! every instant. They combine with AND ([`all_hold`]).
//!
//! They never make a reminder wait: a trigger outside them simply passes.
//! A condition that can't be checked counts as met (spec: Reminders →
//! Triggers, conditions and evaluation): daylight and darkness with no home
//! location set.

use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::place::Place;
use crate::sun;

/// One time-based condition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Condition {
    /// On these days of the week, as `MO`..`SU`, by the calendar day the
    /// trigger falls on.
    Days { days: Vec<String> },
    /// From a time of day (`22:00`) until another (`06:00`): the start is in
    /// and the end is out. It crosses midnight when the end is earlier.
    Window { from: String, to: String },
    /// From a date to a date, both in, such as `2026-12-20` to `2027-01-05`.
    Dates { from: String, to: String },
    /// Every year from a day to a day, both in, such as `06-01` to `08-31`
    /// for summer. It wraps over the new year when the end is earlier
    /// (`11-01` to `02-28`).
    Season { from: String, to: String },
    /// While the Sun is up at home.
    Daylight,
    /// While the Sun is down at home.
    Darkness,
}

fn weekday(code: &str) -> Option<Weekday> {
    Some(match code {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        _ => return None,
    })
}

/// Minutes after midnight of `HH:MM`.
fn minutes_of(s: &str) -> Option<i32> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (i32, i32) = (h.parse().ok()?, m.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

/// `(month, day)` of `MM-DD`, which must be a day some year has (29 February
/// is one).
fn month_day(s: &str) -> Option<(i8, i8)> {
    let (m, d) = s.split_once('-')?;
    if m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (m, d): (i8, i8) = (m.parse().ok()?, d.parse().ok()?);
    Date::new(2024, m, d).ok().map(|_| (m, d))
}

fn parse_date(s: &str) -> Option<Date> {
    s.parse().ok()
}

impl Condition {
    /// Whether the condition can be used, or why not.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Condition::Days { days } => {
                if days.is_empty() {
                    return Err("choose at least one day".into());
                }
                if let Some(bad) = days.iter().find(|d| weekday(d).is_none()) {
                    return Err(format!("{bad} is not a day of the week"));
                }
            }
            Condition::Window { from, to } => {
                let (Some(a), Some(b)) = (minutes_of(from), minutes_of(to)) else {
                    return Err("a time window takes times such as 08:00".into());
                };
                if a == b {
                    return Err("a time window can't start and end at the same time".into());
                }
            }
            Condition::Dates { from, to } => {
                let (Some(a), Some(b)) = (parse_date(from), parse_date(to)) else {
                    return Err("a date range takes dates such as 2026-12-20".into());
                };
                if b < a {
                    return Err("the date range ends before it starts".into());
                }
            }
            Condition::Season { from, to } => {
                if month_day(from).is_none() || month_day(to).is_none() {
                    return Err("a season takes days such as 06-01".into());
                }
            }
            Condition::Daylight | Condition::Darkness => {}
        }
        Ok(())
    }

    /// Whether the condition needs the home location to be checked.
    pub fn needs_home(&self) -> bool {
        matches!(self, Condition::Daylight | Condition::Darkness)
    }

    /// Whether the condition holds at the instant `at` (Unix seconds), for a
    /// reminder in `zone`, with the user's `home`. Daylight and darkness with
    /// no home can't be checked and count as met. An unreadable condition
    /// (validation refuses these) counts as met too.
    pub fn holds(&self, at: i64, zone: &TimeZone, home: Option<&Place>) -> bool {
        let Ok(ts) = Timestamp::from_second(at) else {
            return true;
        };
        let local = zone.to_datetime(ts);
        match self {
            Condition::Days { days } => days
                .iter()
                .filter_map(|d| weekday(d))
                .any(|d| d == local.date().weekday()),
            Condition::Window { from, to } => {
                let (Some(a), Some(b)) = (minutes_of(from), minutes_of(to)) else {
                    return true;
                };
                let t: Time = local.time();
                let now = i32::from(t.hour()) * 60 + i32::from(t.minute());
                if a < b {
                    a <= now && now < b
                } else {
                    now >= a || now < b
                }
            }
            Condition::Dates { from, to } => {
                let (Some(a), Some(b)) = (parse_date(from), parse_date(to)) else {
                    return true;
                };
                (a..=b).contains(&local.date())
            }
            Condition::Season { from, to } => {
                let (Some(a), Some(b)) = (month_day(from), month_day(to)) else {
                    return true;
                };
                let d = (local.date().month(), local.date().day());
                if a <= b {
                    a <= d && d <= b
                } else {
                    d >= a || d <= b
                }
            }
            Condition::Daylight => {
                home.is_none_or(|h| sun::is_daylight(at, h.latitude, h.longitude))
            }
            Condition::Darkness => {
                home.is_none_or(|h| !sun::is_daylight(at, h.latitude, h.longitude))
            }
        }
    }
}

/// Whether every condition holds at `at`: they combine with AND, and none is
/// always true.
pub fn all_hold(conditions: &[Condition], at: i64, zone: &TimeZone, home: Option<&Place>) -> bool {
    conditions.iter().all(|c| c.holds(at, zone, home))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::zone;

    fn utc(s: &str) -> i64 {
        s.parse::<Timestamp>().unwrap().as_second()
    }

    fn days(codes: &[&str]) -> Condition {
        Condition::Days {
            days: codes.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn window(from: &str, to: &str) -> Condition {
        Condition::Window {
            from: from.into(),
            to: to.into(),
        }
    }

    fn london() -> Place {
        Place::home(51.5074, -0.1278).unwrap()
    }

    #[test]
    fn days_are_the_local_calendar_day() {
        let weekdays = days(&["MO", "TU", "WE", "TH", "FR"]);
        let utc_zone = zone("UTC").unwrap();
        // 2024-06-21 is a Friday, the 22nd a Saturday.
        assert!(weekdays.holds(utc("2024-06-21T12:00:00Z"), &utc_zone, None));
        assert!(!weekdays.holds(utc("2024-06-22T12:00:00Z"), &utc_zone, None));
        // 23:30 on Friday in UTC is already Saturday in Tokyo.
        let tokyo = zone("Asia/Tokyo").unwrap();
        assert!(!weekdays.holds(utc("2024-06-21T23:30:00Z"), &tokyo, None));
        assert!(weekdays.holds(utc("2024-06-21T23:30:00Z"), &utc_zone, None));
    }

    #[test]
    fn a_window_includes_its_start_and_excludes_its_end() {
        let z = zone("UTC").unwrap();
        let w = window("08:00", "20:00");
        assert!(!w.holds(utc("2024-06-21T07:59:00Z"), &z, None));
        assert!(w.holds(utc("2024-06-21T08:00:00Z"), &z, None));
        assert!(w.holds(utc("2024-06-21T19:59:59Z"), &z, None));
        assert!(!w.holds(utc("2024-06-21T20:00:00Z"), &z, None));
    }

    #[test]
    fn a_window_can_cross_midnight() {
        let z = zone("UTC").unwrap();
        let w = window("22:00", "06:00");
        assert!(w.holds(utc("2024-06-21T23:00:00Z"), &z, None));
        assert!(w.holds(utc("2024-06-22T00:00:00Z"), &z, None));
        assert!(w.holds(utc("2024-06-22T05:59:00Z"), &z, None));
        assert!(!w.holds(utc("2024-06-22T06:00:00Z"), &z, None));
        assert!(!w.holds(utc("2024-06-21T21:59:00Z"), &z, None));
    }

    #[test]
    fn a_window_is_read_in_the_zone() {
        let w = window("08:00", "20:00");
        let ny = zone("America/New_York").unwrap();
        // 13:00 UTC is 09:00 in New York in summer, and 08:00 in winter.
        assert!(w.holds(utc("2024-07-01T12:00:00Z"), &ny, None)); // 08:00 EDT
        assert!(!w.holds(utc("2024-07-01T11:59:00Z"), &ny, None));
        assert!(!w.holds(utc("2024-01-15T12:59:00Z"), &ny, None)); // 07:59 EST
        assert!(w.holds(utc("2024-01-15T13:00:00Z"), &ny, None));
    }

    #[test]
    fn a_window_follows_the_clock_across_a_daylight_saving_change() {
        // New York springs forward on 2024-03-10 at 02:00. A window from
        // 01:30 to 03:30 is in by the wall clock: 06:30 UTC is 01:30 EST and
        // 07:30 UTC is 03:30 EDT, so an hour and a half of real time.
        let ny = zone("America/New_York").unwrap();
        let w = window("01:30", "03:30");
        assert!(!w.holds(utc("2024-03-10T06:29:00Z"), &ny, None));
        assert!(w.holds(utc("2024-03-10T06:30:00Z"), &ny, None));
        assert!(w.holds(utc("2024-03-10T07:29:00Z"), &ny, None)); // 03:29 EDT
        assert!(!w.holds(utc("2024-03-10T07:30:00Z"), &ny, None));
        // On the fall-back night, 01:30 happens twice: both are in a window
        // that spans it.
        let w = window("01:00", "02:00");
        assert!(w.holds(utc("2024-11-03T05:30:00Z"), &ny, None)); // 01:30 EDT
        assert!(w.holds(utc("2024-11-03T06:30:00Z"), &ny, None)); // 01:30 EST
        assert!(!w.holds(utc("2024-11-03T07:00:00Z"), &ny, None)); // 02:00 EST
    }

    #[test]
    fn a_date_range_includes_both_ends() {
        let z = zone("UTC").unwrap();
        let c = Condition::Dates {
            from: "2026-12-20".into(),
            to: "2027-01-05".into(),
        };
        assert!(!c.holds(utc("2026-12-19T23:59:59Z"), &z, None));
        assert!(c.holds(utc("2026-12-20T00:00:00Z"), &z, None));
        assert!(c.holds(utc("2027-01-05T23:59:59Z"), &z, None));
        assert!(!c.holds(utc("2027-01-06T00:00:00Z"), &z, None));
    }

    #[test]
    fn a_season_repeats_every_year_and_can_wrap_the_new_year() {
        let z = zone("UTC").unwrap();
        let summer = Condition::Season {
            from: "06-01".into(),
            to: "08-31".into(),
        };
        assert!(summer.holds(utc("2024-06-01T00:00:00Z"), &z, None));
        assert!(summer.holds(utc("2031-08-31T23:59:00Z"), &z, None));
        assert!(!summer.holds(utc("2024-05-31T23:59:00Z"), &z, None));
        assert!(!summer.holds(utc("2024-09-01T00:00:00Z"), &z, None));
        let winter = Condition::Season {
            from: "11-01".into(),
            to: "02-29".into(),
        };
        assert!(winter.holds(utc("2024-11-01T10:00:00Z"), &z, None));
        assert!(winter.holds(utc("2025-01-15T10:00:00Z"), &z, None));
        assert!(winter.holds(utc("2025-02-28T10:00:00Z"), &z, None));
        assert!(winter.holds(utc("2024-02-29T10:00:00Z"), &z, None));
        assert!(!winter.holds(utc("2025-03-01T10:00:00Z"), &z, None));
        assert!(!winter.holds(utc("2024-10-31T10:00:00Z"), &z, None));
        assert!(Condition::Season {
            from: "02-30".into(),
            to: "03-01".into()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn daylight_and_darkness_are_opposites_at_home() {
        let z = zone("Europe/London").unwrap();
        let home = london();
        let noon = utc("2024-06-21T11:00:00Z");
        let midnight = utc("2024-06-21T23:30:00Z");
        assert!(Condition::Daylight.holds(noon, &z, Some(&home)));
        assert!(!Condition::Darkness.holds(noon, &z, Some(&home)));
        assert!(!Condition::Daylight.holds(midnight, &z, Some(&home)));
        assert!(Condition::Darkness.holds(midnight, &z, Some(&home)));
    }

    #[test]
    fn daylight_and_darkness_in_the_polar_day_and_night() {
        let z = zone("Europe/Oslo").unwrap();
        let tromso = Place::home(69.6492, 18.9553).unwrap();
        let midnight_sun = utc("2024-06-21T22:00:00Z"); // midnight in Oslo
        assert!(Condition::Daylight.holds(midnight_sun, &z, Some(&tromso)));
        assert!(!Condition::Darkness.holds(midnight_sun, &z, Some(&tromso)));
        let polar_night = utc("2024-12-21T11:00:00Z"); // midday
        assert!(!Condition::Daylight.holds(polar_night, &z, Some(&tromso)));
        assert!(Condition::Darkness.holds(polar_night, &z, Some(&tromso)));
    }

    #[test]
    fn without_a_home_daylight_and_darkness_count_as_met() {
        let z = zone("UTC").unwrap();
        let t = utc("2024-06-21T23:30:00Z");
        assert!(Condition::Daylight.holds(t, &z, None));
        assert!(Condition::Darkness.holds(t, &z, None));
        assert!(Condition::Daylight.needs_home() && !days(&["MO"]).needs_home());
    }

    #[test]
    fn conditions_combine_with_and_and_none_holds() {
        let z = zone("UTC").unwrap();
        let t = utc("2024-06-21T12:00:00Z"); // Friday noon
        assert!(all_hold(&[], t, &z, None));
        let both = [days(&["FR"]), window("08:00", "13:00")];
        assert!(all_hold(&both, t, &z, None));
        let one_fails = [days(&["FR"]), window("13:00", "14:00")];
        assert!(!all_hold(&one_fails, t, &z, None));
    }

    #[test]
    fn evaluation_is_the_same_wherever_it_is_made() {
        // A pure function of the instant, the zone and the home: the same
        // inputs give the same answer every time.
        let z = zone("Europe/London").unwrap();
        let home = london();
        let cs = [
            days(&["MO", "TU", "WE", "TH", "FR"]),
            window("07:00", "19:00"),
            Condition::Daylight,
        ];
        let mut t = utc("2024-06-01T00:00:00Z");
        while t < utc("2024-06-15T00:00:00Z") {
            assert_eq!(
                all_hold(&cs, t, &z, Some(&home)),
                all_hold(&cs, t, &z, Some(&home))
            );
            t += 600;
        }
    }

    #[test]
    fn bad_conditions_are_refused() {
        assert!(days(&[]).validate().is_err());
        assert!(days(&["XX"]).validate().is_err());
        assert!(window("8:00", "20:00").validate().is_err());
        assert!(window("25:00", "20:00").validate().is_err());
        assert!(window("08:00", "08:00").validate().is_err());
        assert!(Condition::Dates {
            from: "2026-12-20".into(),
            to: "2026-12-19".into()
        }
        .validate()
        .is_err());
        assert!(Condition::Dates {
            from: "nope".into(),
            to: "2026-12-19".into()
        }
        .validate()
        .is_err());
        assert!(window("22:00", "06:00").validate().is_ok());
        assert!(days(&["SA", "SU"]).validate().is_ok());
    }
}
