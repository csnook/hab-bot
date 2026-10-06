//! Sun events, calculated on the device with no network (spec: Sources →
//! Time → Sun events; ADR 0012).
//!
//! The algorithm is NOAA's solar calculator, which is Jean Meeus's
//! *Astronomical Algorithms* (chapters 7, 12, 13, 15 and 25) in the
//! simplified form NOAA publishes: the Sun's apparent position from the
//! Julian century, the equation of time and the declination, then the hour
//! angle at which the Sun's centre is a given angle below the horizon. It is
//! good to about a minute between 60° north and south, and a little less
//! near the polar circles.
//!
//! - **Sunrise and sunset** are when the Sun's upper edge meets the horizon,
//!   allowing for refraction: the centre is 0.833° below it.
//! - **Civil dawn and dusk** are when the centre is 6° below it.
//! - **A day** is the solar day around local solar noon at the place: the
//!   calendar date there, to within a fraction of a time zone. Its sunrise
//!   may fall on the previous UTC date (Sydney) and its sunset on the next
//!   (Honolulu); the instant is what counts.
//! - **A day without the event** (the midnight sun, the polar night, and for
//!   civil twilight the white nights) has none: [`event_time`] is `None` and
//!   nothing fires.

use jiff::civil::Date;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

const DAY_SECONDS: i64 = 86_400;
/// Zenith angle of sunrise and sunset: 90° plus refraction and the Sun's
/// radius.
const ZENITH_SUN: f64 = 90.833;
/// Zenith angle of civil dawn and dusk.
const ZENITH_CIVIL: f64 = 96.0;

/// The most a sun event can be offset by, in minutes: half a day either way.
pub const MAX_OFFSET_MINUTES: i32 = 12 * 60;

/// A moment in the Sun's day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SunEvent {
    Sunrise,
    Sunset,
    /// First light, before sunrise.
    CivilDawn,
    /// Last light, after sunset.
    CivilDusk,
}

impl SunEvent {
    fn zenith(self) -> f64 {
        match self {
            SunEvent::Sunrise | SunEvent::Sunset => ZENITH_SUN,
            SunEvent::CivilDawn | SunEvent::CivilDusk => ZENITH_CIVIL,
        }
    }

    fn rising(self) -> bool {
        matches!(self, SunEvent::Sunrise | SunEvent::CivilDawn)
    }
}

/// A sun-event trigger: a sun event and an offset from it, such as "30
/// minutes before sunset" (`offset_minutes` -30).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SunTrigger {
    pub event: SunEvent,
    /// Minutes after the event; negative is before it.
    pub offset_minutes: i32,
}

impl SunTrigger {
    pub fn validate(&self) -> Result<(), String> {
        if self.offset_minutes.abs() > MAX_OFFSET_MINUTES {
            return Err("a sun event can be offset by at most 12 hours".into());
        }
        Ok(())
    }

    /// The instant on the solar day of `date` at the place, if the event
    /// happens that day.
    pub fn on(&self, date: Date, latitude: f64, longitude: f64) -> Option<i64> {
        event_time(date, self.event, latitude, longitude)
            .map(|t| t + i64::from(self.offset_minutes) * 60)
    }

    /// The instants after `after` and up to `until`, earliest first, at a
    /// place.
    pub fn instants(&self, latitude: f64, longitude: f64, after: i64, until: i64) -> Vec<i64> {
        let mut out = Vec::new();
        if until <= after {
            return out;
        }
        // The solar day of a date is within a day of its UTC date, and the
        // offset can add half a day more.
        let margin = 2 + i64::from(self.offset_minutes.abs()) / (24 * 60);
        let (Some(first), Some(last)) = (utc_date(after), utc_date(until)) else {
            return out;
        };
        let (first, last) = (
            first.saturating_sub(margin + 1),
            last.saturating_add(margin + 1),
        );
        for day in first..=last {
            let Some(date) = date_of_unix_day(day) else {
                continue;
            };
            if let Some(t) = self.on(date, latitude, longitude) {
                if t > after && t <= until {
                    out.push(t);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Days since 1970-01-01 of the UTC date containing `t`.
fn utc_date(t: i64) -> Option<i64> {
    Some(t.div_euclid(DAY_SECONDS))
}

fn date_of_unix_day(day: i64) -> Option<Date> {
    Timestamp::from_second(day.checked_mul(DAY_SECONDS)?)
        .ok()
        .map(|ts| ts.to_zoned(jiff::tz::TimeZone::UTC).date())
}

fn unix_day_of(date: Date) -> i64 {
    date.to_zoned(jiff::tz::TimeZone::UTC)
        .map_or(0, |z| z.timestamp().as_second().div_euclid(DAY_SECONDS))
}

/// The Sun's declination (radians) and the equation of time (minutes) at a
/// Julian day (NOAA's solar calculator).
fn position(jd: f64) -> (f64, f64) {
    let t = (jd - 2_451_545.0) / 36_525.0;
    let l0 = (280.46646 + t * (36_000.769_83 + t * 0.000_303_2)).rem_euclid(360.0);
    let m = 357.52911 + t * (35_999.050_29 - 0.000_153_7 * t);
    let e = 0.016_708_634 - t * (0.000_042_037 + 0.000_000_126_7 * t);
    let (mr, l0r) = (m.to_radians(), l0.to_radians());
    let centre = mr.sin() * (1.914_602 - t * (0.004_817 + 0.000_014 * t))
        + (2.0 * mr).sin() * (0.019_993 - 0.000_101 * t)
        + (3.0 * mr).sin() * 0.000_289;
    let true_long = l0 + centre;
    let omega = 125.04 - 1934.136 * t;
    let apparent = true_long - 0.00569 - 0.00478 * omega.to_radians().sin();
    let mean_obliquity =
        23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001_813))) / 60.0) / 60.0;
    let obliquity = (mean_obliquity + 0.00256 * omega.to_radians().cos()).to_radians();
    let declination = (obliquity.sin() * apparent.to_radians().sin()).asin();
    let y = (obliquity / 2.0).tan().powi(2);
    let equation = 4.0
        * (y * (2.0 * l0r).sin() - 2.0 * e * mr.sin() + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos()
            - 0.5 * y * y * (4.0 * l0r).sin()
            - 1.25 * e * e * (2.0 * mr).sin())
        .to_degrees();
    (declination, equation)
}

/// The hour angle (degrees) at which the Sun is at `zenith`, or `None` if it
/// never gets that low or high that day.
fn hour_angle(latitude: f64, declination: f64, zenith: f64) -> Option<f64> {
    let lat = latitude.to_radians();
    let c = (zenith.to_radians().cos() - lat.sin() * declination.sin())
        / (lat.cos() * declination.cos());
    (-1.0..=1.0).contains(&c).then(|| c.acos().to_degrees())
}

/// The instant (Unix seconds) of `event` on the solar day of `date` at a
/// place, or `None` on a day it doesn't happen. `latitude` is north positive
/// and `longitude` east positive, in degrees.
pub fn event_time(date: Date, event: SunEvent, latitude: f64, longitude: f64) -> Option<i64> {
    let day = unix_day_of(date);
    let jd0 = day as f64 + 2_440_587.5;
    // Whether the event happens is judged at solar noon of that day, so a
    // day's answer doesn't depend on where the iteration lands.
    let noon = 720.0 - 4.0 * longitude;
    let (declination, _) = position(jd0 + noon / 1440.0);
    hour_angle(latitude, declination, event.zenith())?;
    let mut minutes = noon;
    for _ in 0..4 {
        let (declination, equation) = position(jd0 + minutes / 1440.0);
        let ha = hour_angle(latitude, declination, event.zenith()).unwrap_or(0.0);
        minutes = if event.rising() {
            720.0 - 4.0 * (longitude + ha) - equation
        } else {
            720.0 - 4.0 * (longitude - ha) - equation
        };
    }
    Some(day * DAY_SECONDS + (minutes * 60.0).round() as i64)
}

/// Whether the Sun is up at `at` (Unix seconds) at a place: its centre above
/// the sunrise and sunset angle. Daylight, as the condition means it; its
/// opposite is darkness. Polar day and night come out right with no special
/// case.
pub fn is_daylight(at: i64, latitude: f64, longitude: f64) -> bool {
    let jd = at as f64 / 86_400.0 + 2_440_587.5;
    let (declination, equation) = position(jd);
    let minutes = at.rem_euclid(DAY_SECONDS) as f64 / 60.0;
    let solar = (minutes + equation + 4.0 * longitude).rem_euclid(1440.0);
    let hour = (solar / 4.0 - 180.0).to_radians();
    let lat = latitude.to_radians();
    let cos_zenith = lat.sin() * declination.sin() + lat.cos() * declination.cos() * hour.cos();
    cos_zenith.clamp(-1.0, 1.0).acos().to_degrees() < ZENITH_SUN
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const LONDON: (f64, f64) = (51.5074, -0.1278);
    const NEW_YORK: (f64, f64) = (40.7128, -74.0060);
    const SYDNEY: (f64, f64) = (-33.8688, 151.2093);
    const QUITO: (f64, f64) = (-0.1807, -78.4678);
    const TROMSO: (f64, f64) = (69.6492, 18.9553);
    const KIRITIMATI: (f64, f64) = (1.8721, -157.4278);

    fn utc(s: &str) -> i64 {
        s.parse::<Timestamp>().unwrap().as_second()
    }

    /// Within `minutes` of the published time.
    fn near(got: Option<i64>, want: &str, minutes: i64) {
        let got = got.unwrap_or_else(|| panic!("no event, wanted {want}"));
        let want = utc(want);
        assert!(
            (got - want).abs() <= minutes * 60,
            "got {} wanted {want} ({} s apart)",
            Timestamp::from_second(got).unwrap(),
            got - want
        );
    }

    fn at(place: (f64, f64), d: Date, event: SunEvent) -> Option<i64> {
        event_time(d, event, place.0, place.1)
    }

    // The reference times are the Sun's centre at -0.833° (-6° for civil
    // twilight) from PyEphem (libastro, the USNO's algorithms: an
    // independent implementation), checked against published almanac times
    // such as timeanddate.com's for London on 2024-06-21: sunrise 04:43 BST,
    // sunset 21:21 BST.

    #[test]
    fn london_at_the_solstices() {
        use SunEvent::*;
        let d = date(2024, 6, 21);
        near(at(LONDON, d, Sunrise), "2024-06-21T03:43:12Z", 2);
        near(at(LONDON, d, Sunset), "2024-06-21T20:21:38Z", 2);
        near(at(LONDON, d, CivilDawn), "2024-06-21T02:55:25Z", 2);
        near(at(LONDON, d, CivilDusk), "2024-06-21T21:09:24Z", 2);
        let d = date(2024, 12, 21);
        near(at(LONDON, d, Sunrise), "2024-12-21T08:03:59Z", 2);
        near(at(LONDON, d, Sunset), "2024-12-21T15:53:36Z", 2);
        near(at(LONDON, d, CivilDawn), "2024-12-21T07:23:38Z", 2);
        near(at(LONDON, d, CivilDusk), "2024-12-21T16:33:57Z", 2);
    }

    #[test]
    fn the_days_the_clocks_change_are_the_same_instants() {
        use SunEvent::*;
        // The calculation is in UTC, so a daylight-saving change moves
        // nothing: London springs forward on 2024-03-31 and falls back on
        // 2024-10-27, New York on 2024-03-10 and 2024-11-03.
        near(
            at(LONDON, date(2024, 3, 31), Sunrise),
            "2024-03-31T05:37:14Z",
            2,
        );
        near(
            at(LONDON, date(2024, 3, 31), Sunset),
            "2024-03-31T18:32:50Z",
            2,
        );
        near(
            at(LONDON, date(2024, 10, 27), Sunrise),
            "2024-10-27T06:45:57Z",
            2,
        );
        near(
            at(LONDON, date(2024, 10, 27), Sunset),
            "2024-10-27T16:41:55Z",
            2,
        );
        near(
            at(NEW_YORK, date(2024, 3, 10), Sunrise),
            "2024-03-10T11:14:54Z",
            2,
        );
        near(
            at(NEW_YORK, date(2024, 3, 10), Sunset),
            "2024-03-10T22:57:55Z",
            2,
        );
        near(
            at(NEW_YORK, date(2024, 11, 3), Sunrise),
            "2024-11-03T11:29:20Z",
            2,
        );
        near(
            at(NEW_YORK, date(2024, 11, 3), Sunset),
            "2024-11-03T21:49:18Z",
            2,
        );
    }

    #[test]
    fn a_day_can_start_and_end_on_other_utc_dates() {
        use SunEvent::*;
        near(
            at(NEW_YORK, date(2024, 6, 21), Sunrise),
            "2024-06-21T09:25:07Z",
            2,
        );
        near(
            at(NEW_YORK, date(2024, 6, 21), Sunset),
            "2024-06-22T00:30:49Z",
            2,
        );
        near(
            at(SYDNEY, date(2024, 12, 21), Sunrise),
            "2024-12-20T18:40:51Z",
            2,
        );
        near(
            at(SYDNEY, date(2024, 12, 21), Sunset),
            "2024-12-21T09:05:38Z",
            2,
        );
        near(
            at(KIRITIMATI, date(2024, 6, 21), Sunrise),
            "2024-06-21T16:24:47Z",
            2,
        );
        near(
            at(KIRITIMATI, date(2024, 6, 21), Sunset),
            "2024-06-22T04:38:39Z",
            2,
        );
    }

    #[test]
    fn the_equator_and_the_southern_hemisphere() {
        use SunEvent::*;
        near(
            at(QUITO, date(2024, 3, 20), Sunrise),
            "2024-03-20T11:17:52Z",
            2,
        );
        near(
            at(QUITO, date(2024, 3, 20), Sunset),
            "2024-03-20T23:24:21Z",
            2,
        );
    }

    #[test]
    fn polar_day_and_night_have_no_sunrise_or_sunset() {
        use SunEvent::*;
        // Tromsø has the midnight sun from about 18 May to 26 July, and the
        // polar night from about 27 November to 15 January. On the days at
        // each edge the Sun grazes the horizon, within what the algorithm
        // can tell apart, so those days aren't asserted.
        for d in [date(2024, 6, 21), date(2024, 5, 20), date(2024, 7, 20)] {
            assert_eq!(at(TROMSO, d, Sunrise), None, "{d}");
            assert_eq!(at(TROMSO, d, Sunset), None, "{d}");
        }
        for d in [date(2024, 12, 21), date(2024, 11, 29), date(2024, 1, 12)] {
            assert_eq!(at(TROMSO, d, Sunrise), None, "{d}");
            assert_eq!(at(TROMSO, d, Sunset), None, "{d}");
        }
        // The days either side of the changes have both.
        near(
            at(TROMSO, date(2024, 1, 16), Sunrise),
            "2024-01-16T10:20:51Z",
            3,
        );
        near(
            at(TROMSO, date(2024, 1, 16), Sunset),
            "2024-01-16T11:27:22Z",
            3,
        );
        near(
            at(TROMSO, date(2024, 11, 26), Sunrise),
            "2024-11-26T10:05:57Z",
            3,
        );
        near(
            at(TROMSO, date(2024, 11, 26), Sunset),
            "2024-11-26T10:56:28Z",
            3,
        );
        assert!(at(TROMSO, date(2024, 5, 10), Sunrise).is_some());
        assert!(at(TROMSO, date(2024, 8, 10), Sunset).is_some());
    }

    #[test]
    fn civil_twilight_survives_the_polar_night_and_vanishes_in_the_white_nights() {
        use SunEvent::*;
        // The Sun is up for no time at the winter solstice, but civil
        // twilight lasts for hours.
        near(
            at(TROMSO, date(2024, 12, 21), CivilDawn),
            "2024-12-21T08:31:31Z",
            3,
        );
        near(
            at(TROMSO, date(2024, 12, 21), CivilDusk),
            "2024-12-21T12:53:21Z",
            3,
        );
        // At midsummer it never gets 6° below the horizon.
        assert_eq!(at(TROMSO, date(2024, 6, 21), CivilDawn), None);
        assert_eq!(at(TROMSO, date(2024, 6, 21), CivilDusk), None);
        // London in midsummer still has civil twilight on both ends.
        assert!(at(LONDON, date(2024, 6, 21), CivilDawn).is_some());
    }

    #[test]
    fn an_offset_moves_the_instant() {
        let d = date(2024, 6, 21);
        let sunset = at(LONDON, d, SunEvent::Sunset).unwrap();
        let before = SunTrigger {
            event: SunEvent::Sunset,
            offset_minutes: -30,
        };
        assert_eq!(before.on(d, LONDON.0, LONDON.1), Some(sunset - 30 * 60));
        let after = SunTrigger {
            event: SunEvent::Sunset,
            offset_minutes: 45,
        };
        assert_eq!(after.on(d, LONDON.0, LONDON.1), Some(sunset + 45 * 60));
        assert!(SunTrigger {
            event: SunEvent::Sunset,
            offset_minutes: 13 * 60
        }
        .validate()
        .is_err());
    }

    #[test]
    fn instants_are_found_in_a_range_and_skip_days_without_the_event() {
        let t = SunTrigger {
            event: SunEvent::Sunset,
            offset_minutes: 0,
        };
        // A week in London is seven sunsets, earliest first, each once.
        let after = utc("2024-06-20T00:00:00Z");
        let until = utc("2024-06-27T00:00:00Z");
        let v = t.instants(LONDON.0, LONDON.1, after, until);
        assert_eq!(v.len(), 7);
        assert!(v.windows(2).all(|w| w[0] < w[1]));
        // The range is (after, until]: an instant at `until` counts, at
        // `after` doesn't.
        let last = *v.last().unwrap();
        assert_eq!(t.instants(LONDON.0, LONDON.1, after, last).len(), 7);
        assert_eq!(t.instants(LONDON.0, LONDON.1, after, last - 1).len(), 6);
        assert_eq!(t.instants(LONDON.0, LONDON.1, v[0], until).len(), 6);
        // In the midnight sun, none.
        assert!(t
            .instants(
                TROMSO.0,
                TROMSO.1,
                utc("2024-06-01T00:00:00Z"),
                utc("2024-07-10T00:00:00Z")
            )
            .is_empty());
        // A range spanning the end of the polar night finds only the days
        // that have one.
        let none = t.instants(
            TROMSO.0,
            TROMSO.1,
            utc("2024-01-05T00:00:00Z"),
            utc("2024-01-13T00:00:00Z"),
        );
        assert!(none.is_empty());
        let v = t.instants(
            TROMSO.0,
            TROMSO.1,
            utc("2024-01-05T00:00:00Z"),
            utc("2024-01-22T23:00:00Z"),
        );
        // The last few days of the range have a sunset each, the 18th to the
        // 22nd, and the first week none.
        assert!((5..=8).contains(&v.len()), "{}", v.len());
        assert!(v.iter().all(|t| *t > utc("2024-01-13T00:00:00Z")));
    }

    #[test]
    fn a_negative_offset_never_finds_the_same_instant_twice() {
        let t = SunTrigger {
            event: SunEvent::Sunrise,
            offset_minutes: -12 * 60,
        };
        let v = t.instants(
            SYDNEY.0,
            SYDNEY.1,
            utc("2024-12-01T00:00:00Z"),
            utc("2024-12-11T00:00:00Z"),
        );
        assert_eq!(v.len(), 10);
        let mut s = v.clone();
        s.dedup();
        assert_eq!(s, v);
    }

    #[test]
    fn daylight_follows_sunrise_and_sunset() {
        let d = date(2024, 6, 21);
        let rise = at(LONDON, d, SunEvent::Sunrise).unwrap();
        let set = at(LONDON, d, SunEvent::Sunset).unwrap();
        assert!(!is_daylight(rise - 120, LONDON.0, LONDON.1));
        assert!(is_daylight(rise + 120, LONDON.0, LONDON.1));
        assert!(is_daylight(set - 120, LONDON.0, LONDON.1));
        assert!(!is_daylight(set + 120, LONDON.0, LONDON.1));
        // Noon and midnight, on the far side of the world too.
        assert!(is_daylight(utc("2024-06-21T12:00:00Z"), LONDON.0, LONDON.1));
        assert!(!is_daylight(
            utc("2024-06-21T00:00:00Z"),
            LONDON.0,
            LONDON.1
        ));
        assert!(is_daylight(utc("2024-12-21T00:00:00Z"), SYDNEY.0, SYDNEY.1));
    }

    #[test]
    fn daylight_in_the_polar_day_and_night() {
        for h in 0..24 {
            let t = utc("2024-06-21T00:00:00Z") + h * 3600;
            assert!(is_daylight(t, TROMSO.0, TROMSO.1), "midnight sun at {h}");
            let t = utc("2024-12-21T00:00:00Z") + h * 3600;
            assert!(!is_daylight(t, TROMSO.0, TROMSO.1), "polar night at {h}");
        }
    }
}
