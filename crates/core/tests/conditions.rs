//! Time-based conditions, sun events and the home location (#48): what is
//! predicted and fired, how it interacts with pausing, expiry, the alerter and
//! the inbox, and two devices agreeing. Time is passed in, so every test says
//! what the clock reads.

use hab_core::{
    Alerter, Change, Command, Condition, Core, EditReminder, Error, Event, Hlc, Place, Priority,
    Schedule, SunEvent, SunTrigger, FORMAT_VERSION,
};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

const USER: &str = "u1";
const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;
const MIN: i64 = 60;

const LONDON: (f64, f64) = (51.5074, -0.1278);
const TROMSO: (f64, f64) = (69.6492, 18.9553);

fn at(local: &str) -> i64 {
    at_in("UTC", local)
}

fn at_in(zone: &str, local: &str) -> i64 {
    let dt: DateTime = local.parse().unwrap();
    TimeZone::get(zone)
        .unwrap()
        .to_zoned(dt)
        .unwrap()
        .timestamp()
        .as_second()
}

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

fn daily_at(times: &[&str]) -> Vec<Schedule> {
    times
        .iter()
        .map(|t| Schedule {
            start: format!("2024-01-01T{t}:00"),
            rule: "FREQ=DAILY".into(),
        })
        .collect()
}

fn list(c: &Core) -> String {
    c.personal_list_id().to_string()
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

fn sun(event: SunEvent, offset_minutes: i32) -> SunTrigger {
    SunTrigger {
        event,
        offset_minutes,
    }
}

/// A reminder in UTC on `times` every day, with `conditions`, made at `now`.
fn make(c: &mut Core, times: &[&str], conditions: Vec<Condition>, now: i64) -> String {
    let l = list(c);
    c.create_repeating_reminder_in(
        &l,
        "Water the garden",
        daily_at(times),
        vec![],
        conditions,
        Some("UTC"),
        now,
    )
    .unwrap()
}

/// Ticks through `from..=to` in steps of `step`, returning every scheduled
/// time that fired.
fn fires(c: &mut Core, from: i64, to: i64, step: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let mut t = from;
    while t <= to {
        for f in c.tick(t).unwrap() {
            let at: i64 = f.occurrence_id.rsplit('@').next().unwrap().parse().unwrap();
            out.push(at);
        }
        t += step;
    }
    out
}

fn scheduled(c: &Core, now: i64, until: i64) -> Vec<i64> {
    c.expected(now, until)
        .into_iter()
        .map(|e| e.scheduled_at)
        .collect()
}

// ---- Days, windows, dates and seasons ----

#[test]
fn a_weekdays_only_reminder_shows_nothing_on_saturday_and_sunday() {
    let mut c = core();
    // 2026-10-02 is a Friday.
    let now = at("2026-10-02T06:00:00");
    make(
        &mut c,
        &["09:00"],
        vec![days(&["MO", "TU", "WE", "TH", "FR"])],
        now,
    );
    let expected = scheduled(&c, now, at("2026-10-07T00:00:00"));
    assert_eq!(
        expected,
        [
            at("2026-10-02T09:00:00"),
            at("2026-10-05T09:00:00"),
            at("2026-10-06T09:00:00"),
        ]
    );
    assert!(scheduled(&c, at("2026-10-02T10:00:00"), at("2026-10-04T23:59:00")).is_empty());
}

#[test]
fn an_instant_outside_the_conditions_passes_without_firing_or_waiting() {
    let mut c = core();
    let now = at("2026-10-02T06:00:00");
    make(
        &mut c,
        &["09:00"],
        vec![days(&["MO", "TU", "WE", "TH", "FR"])],
        now,
    );
    // Friday fires; the weekend doesn't; Monday does.
    let f = fires(&mut c, now, at("2026-10-06T00:00:00"), 30 * MIN);
    assert_eq!(f, [at("2026-10-02T09:00:00"), at("2026-10-05T09:00:00")]);
    // Nothing was opened for Saturday or Sunday, open, closed or waiting.
    let all = &c.state().occurrences;
    assert_eq!(all.len(), 2);
    assert!(all
        .values()
        .all(|o| o.scheduled_at != at("2026-10-03T09:00:00")));
}

#[test]
fn a_passed_trigger_does_not_expire_the_open_occurrence() {
    let mut c = core();
    let now = at("2026-10-02T06:00:00");
    let rid = make(&mut c, &["09:00"], vec![days(&["FR"])], now);
    c.tick(at("2026-10-02T09:00:00")).unwrap();
    let friday = format!("{rid}@{}", at("2026-10-02T09:00:00"));
    // Eight days of triggers that pass leave Friday open (ADR 0001 only
    // misses an occurrence when the reminder fires again).
    fires(
        &mut c,
        at("2026-10-02T10:00:00"),
        at("2026-10-08T12:00:00"),
        HOUR,
    );
    assert!(c.state().occurrences[&friday].is_open());
    // The next Friday fires and misses it.
    c.tick(at("2026-10-09T09:00:00")).unwrap();
    assert!(!c.state().occurrences[&friday].is_open());
}

#[test]
fn a_time_window_can_cross_midnight() {
    let mut c = core();
    let now = at("2026-10-02T00:00:00");
    make(
        &mut c,
        &["02:00", "12:00", "23:00"],
        vec![window("22:00", "06:00")],
        now,
    );
    let f = fires(&mut c, now, at("2026-10-04T00:00:00"), 30 * MIN);
    assert_eq!(
        f,
        [
            at("2026-10-02T02:00:00"),
            at("2026-10-02T23:00:00"),
            at("2026-10-03T02:00:00"),
            at("2026-10-03T23:00:00"),
        ]
    );
}

#[test]
fn a_date_range_and_a_season_bound_what_is_predicted() {
    let mut c = core();
    let now = at("2026-10-02T00:00:00");
    make(
        &mut c,
        &["09:00"],
        vec![Condition::Dates {
            from: "2026-10-10".into(),
            to: "2026-10-12".into(),
        }],
        now,
    );
    assert_eq!(
        scheduled(&c, now, at("2026-11-30T00:00:00")),
        [
            at("2026-10-10T09:00:00"),
            at("2026-10-11T09:00:00"),
            at("2026-10-12T09:00:00"),
        ]
    );
    // A season that is months away is still found by the scheduler.
    let mut c = core();
    make(
        &mut c,
        &["09:00"],
        vec![Condition::Season {
            from: "06-01".into(),
            to: "08-31".into(),
        }],
        now,
    );
    assert_eq!(c.next_fire_at(), Some(at("2027-06-01T09:00:00")));
    // And a season that wraps the new year.
    let mut c = core();
    make(
        &mut c,
        &["09:00"],
        vec![Condition::Season {
            from: "11-01".into(),
            to: "02-28".into(),
        }],
        now,
    );
    assert_eq!(c.next_fire_at(), Some(at("2026-11-01T09:00:00")));
}

#[test]
fn conditions_combine_with_and() {
    let mut c = core();
    let now = at("2026-10-02T00:00:00");
    // Weekends, in the morning.
    make(
        &mut c,
        &["08:00", "15:00"],
        vec![days(&["SA", "SU"]), window("06:00", "12:00")],
        now,
    );
    let f = fires(&mut c, now, at("2026-10-06T00:00:00"), HOUR);
    assert_eq!(f, [at("2026-10-03T08:00:00"), at("2026-10-04T08:00:00")]);
}

#[test]
fn conditions_are_read_in_the_reminders_zone_across_a_daylight_saving_change() {
    // Every day at 07:00 London time, only 06:30 to 07:30 London time. The
    // clocks go forward on 2024-03-31; the reminder keeps its local hour,
    // so it passes on every day, though its UTC time moves.
    let mut c = core();
    let now = at_in("Europe/London", "2024-03-28T00:00:00");
    let l = list(&c);
    c.create_repeating_reminder_in(
        &l,
        "Pills",
        daily_at(&["07:00"]),
        vec![],
        vec![window("06:30", "07:30")],
        Some("Europe/London"),
        now,
    )
    .unwrap();
    let f = fires(
        &mut c,
        now,
        at_in("Europe/London", "2024-04-03T00:00:00"),
        30 * MIN,
    );
    let want: Vec<i64> = (28..=31)
        .map(|d| at_in("Europe/London", &format!("2024-03-{d}T07:00:00")))
        .chain((1..=2).map(|d| at_in("Europe/London", &format!("2024-04-0{d}T07:00:00"))))
        .collect();
    assert_eq!(f, want);
    // A UTC window would have dropped the days after the change: 07:00
    // London is 06:00 UTC from the 31st.
    assert_eq!(want[3] - want[2], DAY - HOUR);
}

// ---- Home, daylight and darkness ----

#[test]
fn daylight_and_darkness_use_the_home_location() {
    let mut c = core();
    let now = at("2024-06-20T00:00:00");
    c.set_home(LONDON.0, LONDON.1, now).unwrap();
    // London in June: sunrise about 03:43 UTC, sunset about 20:22.
    let day = make(
        &mut c,
        &["02:00", "12:00", "22:00"],
        vec![Condition::Daylight],
        now,
    );
    let dark = make(
        &mut c,
        &["02:00", "12:00", "22:00"],
        vec![Condition::Darkness],
        now,
    );
    let mut times: Vec<(String, i64)> = Vec::new();
    let mut t = now;
    while t <= at("2024-06-21T23:00:00") {
        for f in c.tick(t).unwrap() {
            let s: i64 = f.occurrence_id.rsplit('@').next().unwrap().parse().unwrap();
            times.push((f.reminder_id, s));
        }
        t += 30 * MIN;
    }
    let of = |id: &String| -> Vec<i64> {
        times
            .iter()
            .filter(|(r, _)| r == id)
            .map(|(_, s)| *s)
            .collect()
    };
    assert_eq!(
        of(&day),
        [at("2024-06-20T12:00:00"), at("2024-06-21T12:00:00")]
    );
    assert_eq!(
        of(&dark),
        [
            at("2024-06-20T02:00:00"),
            at("2024-06-20T22:00:00"),
            at("2024-06-21T02:00:00"),
            at("2024-06-21T22:00:00")
        ]
    );
}

#[test]
fn in_the_midnight_sun_it_is_daylight_all_day_and_never_dark() {
    let mut c = core();
    let now = at("2024-06-20T00:00:00");
    c.set_home(TROMSO.0, TROMSO.1, now).unwrap();
    let day = make(&mut c, &["00:30", "12:00"], vec![Condition::Daylight], now);
    let dark = make(&mut c, &["00:30", "12:00"], vec![Condition::Darkness], now);
    let f = fires(&mut c, now, at("2024-06-20T23:00:00"), HOUR);
    assert_eq!(f.len(), 2, "both daylight times fire, no darkness one does");
    let opened: Vec<_> = c
        .state()
        .occurrences
        .values()
        .map(|o| &o.reminder_id)
        .collect();
    assert!(opened.iter().all(|r| **r == day));
    assert!(!opened.iter().any(|r| **r == dark));
}

#[test]
fn in_the_polar_night_it_is_dark_all_day() {
    let mut c = core();
    let now = at("2024-12-20T00:00:00");
    c.set_home(TROMSO.0, TROMSO.1, now).unwrap();
    make(&mut c, &["12:00"], vec![Condition::Daylight], now);
    let dark = make(&mut c, &["12:00"], vec![Condition::Darkness], now);
    fires(&mut c, now, at("2024-12-21T23:00:00"), HOUR);
    let opened: Vec<_> = c
        .state()
        .occurrences
        .values()
        .map(|o| o.reminder_id.clone())
        .collect();
    assert_eq!(opened, [dark.clone(), dark]);
}

#[test]
fn without_a_home_location_daylight_counts_as_met_and_the_view_says_it_needs_one() {
    let mut c = core();
    let now = at("2024-06-20T00:00:00");
    let rid = make(&mut c, &["23:00"], vec![Condition::Daylight], now);
    // 23:00 is dark in London in June, but with no home it can't be checked.
    assert_eq!(fires(&mut c, now, at("2024-06-20T23:30:00"), HOUR).len(), 1);
    assert!(c.reminder_view(&rid).unwrap().needs_home);
    c.set_home(LONDON.0, LONDON.1, at("2024-06-21T00:00:00"))
        .unwrap();
    assert!(!c.reminder_view(&rid).unwrap().needs_home);
    assert_eq!(
        fires(
            &mut c,
            at("2024-06-21T00:00:00"),
            at("2024-06-21T23:30:00"),
            HOUR
        )
        .len(),
        0
    );
}

#[test]
fn a_reminder_without_such_conditions_never_needs_a_home() {
    let mut c = core();
    let rid = make(&mut c, &["09:00"], vec![days(&["MO"])], 0);
    assert!(!c.reminder_view(&rid).unwrap().needs_home);
}

// ---- Sun events ----

#[test]
fn a_sun_event_fires_at_sunset_with_an_offset_and_counts_from_it() {
    let mut c = core();
    let now = at("2024-06-20T00:00:00");
    c.set_home(LONDON.0, LONDON.1, now).unwrap();
    let l = list(&c);
    let rid = c
        .create_repeating_reminder_in(
            &l,
            "Close the chickens in",
            vec![],
            vec![sun(SunEvent::Sunset, -30)],
            vec![],
            Some("Europe/London"),
            now,
        )
        .unwrap();
    // 30 minutes before the London sunset on 2024-06-20 (about 20:21 UTC).
    let want = hab_core::event_time(
        jiff::civil::date(2024, 6, 20),
        SunEvent::Sunset,
        LONDON.0,
        LONDON.1,
    )
    .unwrap()
        - 30 * MIN;
    assert!((want - at("2024-06-20T19:51:00")).abs() < 120);
    assert_eq!(c.next_fire_at(), Some(want));
    assert!(c.tick(want - 1).unwrap().is_empty());
    let fired = c.tick(want).unwrap();
    assert_eq!(fired[0].occurrence_id, format!("{rid}@{want}"));
    // Overdue and expiry count from the scheduled instant: Medium is
    // overdue an hour later.
    let due = &c.state().occurrences[&fired[0].occurrence_id];
    assert_eq!(due.scheduled_at, want);
    let item = c.inbox(want).due;
    assert_eq!(item.len(), 1);
    assert_eq!(item[0].overdue_at, want + HOUR);
    // And the next one is tomorrow's.
    let next = c.next_fire_at().unwrap();
    assert!(next > want + 23 * HOUR && next < want + 25 * HOUR);
}

#[test]
fn sun_events_are_predicted_for_the_timeline() {
    let mut c = core();
    let now = at("2024-06-20T00:00:00");
    c.set_home(LONDON.0, LONDON.1, now).unwrap();
    let l = list(&c);
    c.create_repeating_reminder_in(
        &l,
        "Dawn walk",
        vec![],
        vec![sun(SunEvent::CivilDawn, 0), sun(SunEvent::Sunrise, 15)],
        vec![days(&["SA", "SU"])],
        None,
        now,
    )
    .unwrap();
    // 2024-06-22 is a Saturday: two instants that day and the next, none
    // on the weekdays between.
    let e = scheduled(&c, now, at("2024-06-25T00:00:00"));
    assert_eq!(e.len(), 4);
    assert!(e.windows(2).all(|w| w[0] < w[1]));
    assert!(e[0] > at("2024-06-22T00:00:00") && e[3] < at("2024-06-23T12:00:00"));
}

#[test]
fn no_firing_on_a_day_the_sun_event_does_not_happen() {
    let mut c = core();
    let now = at("2024-06-01T00:00:00");
    c.set_home(TROMSO.0, TROMSO.1, now).unwrap();
    let l = list(&c);
    c.create_repeating_reminder_in(
        &l,
        "Sunset",
        vec![],
        vec![sun(SunEvent::Sunset, 0)],
        vec![],
        Some("Europe/Oslo"),
        now,
    )
    .unwrap();
    // The midnight sun: nothing fires for weeks and the scheduler has
    // nothing to wake for until the sun sets again in late July.
    assert!(fires(&mut c, now, at("2024-07-10T00:00:00"), 6 * HOUR).is_empty());
    assert!(c.expected(now, at("2024-07-10T00:00:00")).is_empty());
    let next = c.next_fire_at().unwrap();
    assert!(next > at("2024-07-20T00:00:00") && next < at("2024-08-05T00:00:00"));
}

#[test]
fn a_sun_event_with_no_home_waits_for_one_and_does_not_fire_what_passed() {
    let mut c = core();
    let now = at("2024-06-20T00:00:00");
    let l = list(&c);
    let rid = c
        .create_repeating_reminder_in(
            &l,
            "Sunset",
            vec![],
            vec![sun(SunEvent::Sunset, 0)],
            vec![],
            Some("UTC"),
            now,
        )
        .unwrap();
    assert!(c.reminder_view(&rid).unwrap().needs_home);
    assert_eq!(c.next_fire_at(), None);
    assert!(fires(&mut c, now, at("2024-06-20T23:00:00"), HOUR).is_empty());
    // Home is set after the day's sunset: that one has passed and doesn't
    // fire late, tomorrow's does.
    c.set_home(LONDON.0, LONDON.1, at("2024-06-20T23:30:00"))
        .unwrap();
    assert!(c.tick(at("2024-06-20T23:31:00")).unwrap().is_empty());
    let next = c.next_fire_at().unwrap();
    assert!(next > at("2024-06-21T19:00:00") && next < at("2024-06-21T21:30:00"));
    assert!(!c.reminder_view(&rid).unwrap().needs_home);
}

#[test]
fn clearing_the_home_stops_sun_events() {
    let mut c = core();
    let now = at("2024-06-20T00:00:00");
    c.set_home(LONDON.0, LONDON.1, now).unwrap();
    let l = list(&c);
    c.create_repeating_reminder_in(
        &l,
        "Sunset",
        vec![],
        vec![sun(SunEvent::Sunset, 0)],
        vec![],
        Some("UTC"),
        now,
    )
    .unwrap();
    assert!(c.next_fire_at().is_some());
    c.clear_home(now + 1).unwrap();
    assert_eq!(c.home(), None);
    assert_eq!(c.next_fire_at(), None);
}

#[test]
fn sun_events_across_a_daylight_saving_change_follow_the_sun_not_the_clock() {
    // London springs forward on 2024-03-31: sunset is at 18:32 UTC that day,
    // 18:30 the day before, so in local time it jumps an hour.
    let mut c = core();
    let now = at("2024-03-29T00:00:00");
    c.set_home(LONDON.0, LONDON.1, now).unwrap();
    let l = list(&c);
    c.create_repeating_reminder_in(
        &l,
        "Sunset",
        vec![],
        vec![sun(SunEvent::Sunset, 0)],
        vec![],
        Some("Europe/London"),
        now,
    )
    .unwrap();
    let e = scheduled(&c, now, at("2024-04-02T00:00:00"));
    assert_eq!(e.len(), 4);
    let zone = TimeZone::get("Europe/London").unwrap();
    let local = |t: i64| {
        let z = zone.to_datetime(jiff::Timestamp::from_second(t).unwrap());
        (z.hour(), z.minute())
    };
    assert_eq!(local(e[0]).0, 18); // 29th: GMT
    assert_eq!(local(e[1]).0, 18); // 30th: GMT
    assert_eq!(local(e[2]).0, 19); // 31st: BST
    assert_eq!(local(e[3]).0, 19);
    // Within a couple of minutes of the same UTC time either side.
    assert!((e[2] - e[1] - DAY).abs() < 4 * MIN);
}

#[test]
fn a_sun_event_and_a_schedule_can_share_a_reminder() {
    let mut c = core();
    let now = at("2024-06-20T00:00:00");
    c.set_home(LONDON.0, LONDON.1, now).unwrap();
    let l = list(&c);
    c.create_repeating_reminder_in(
        &l,
        "Both",
        daily_at(&["09:00"]),
        vec![sun(SunEvent::Sunset, 0)],
        vec![],
        Some("UTC"),
        now,
    )
    .unwrap();
    assert_eq!(scheduled(&c, now, at("2024-06-21T23:59:00")).len(), 4);
}

// ---- Editing ----

#[test]
fn editing_conditions_and_sun_events_applies_from_then_and_can_be_cleared() {
    let mut c = core();
    let start = at("2026-10-02T06:00:00");
    let rid = make(&mut c, &["09:00"], vec![], start);
    c.edit_reminder(
        &rid,
        EditReminder {
            conditions: Some(vec![days(&["MO"])]),
            ..Default::default()
        },
        start,
    )
    .unwrap();
    let v = c.reminder_view(&rid).unwrap();
    assert_eq!(v.conditions, [days(&["MO"])]);
    assert_eq!(
        scheduled(&c, start, at("2026-10-09T00:00:00")),
        [at("2026-10-05T09:00:00")]
    );
    // Clearing them brings every day back.
    c.edit_reminder(
        &rid,
        EditReminder {
            conditions: Some(vec![]),
            ..Default::default()
        },
        start + 1,
    )
    .unwrap();
    assert_eq!(scheduled(&c, start, at("2026-10-04T00:00:00")).len(), 2);
}

#[test]
fn loosening_a_condition_does_not_fire_what_has_passed() {
    let mut c = core();
    let start = at("2026-10-02T06:00:00");
    let rid = make(&mut c, &["09:00"], vec![days(&["MO"])], start);
    // On Wednesday the user drops the Monday-only condition: Tuesday's 09:00
    // is not fired late, Thursday's is.
    let wed = at("2026-10-07T12:00:00");
    c.edit_reminder(
        &rid,
        EditReminder {
            conditions: Some(vec![]),
            ..Default::default()
        },
        wed,
    )
    .unwrap();
    assert!(c.tick(wed + MIN).unwrap().is_empty());
    assert_eq!(c.next_fire_at(), Some(at("2026-10-08T09:00:00")));
}

#[test]
fn a_sun_event_can_be_added_to_a_schedule_reminder_and_a_sun_only_one_keeps_a_trigger() {
    let mut c = core();
    let now = at("2024-06-20T00:00:00");
    c.set_home(LONDON.0, LONDON.1, now).unwrap();
    let l = list(&c);
    let rid = c
        .create_repeating_reminder_in(
            &l,
            "Sunset",
            vec![],
            vec![sun(SunEvent::Sunset, 0)],
            vec![],
            Some("UTC"),
            now,
        )
        .unwrap();
    let err = c
        .edit_reminder(
            &rid,
            EditReminder {
                suns: Some(vec![]),
                ..Default::default()
            },
            now,
        )
        .unwrap_err();
    assert!(matches!(err, Error::BadSunEvent(_)), "{err}");
    c.edit_reminder(
        &rid,
        EditReminder {
            suns: Some(vec![sun(SunEvent::Sunrise, -10)]),
            schedules: Some(daily_at(&["09:00"])),
            ..Default::default()
        },
        now,
    )
    .unwrap();
    let v = c.reminder_view(&rid).unwrap();
    match v.trigger {
        hab_core::TriggerView::Schedules { schedules, suns } => {
            assert_eq!(schedules.len(), 1);
            assert_eq!(suns, [sun(SunEvent::Sunrise, -10)]);
        }
        t => panic!("{t:?}"),
    }
}

#[test]
fn bad_conditions_sun_events_and_homes_are_refused() {
    let mut c = core();
    let l = list(&c);
    let make_with = |c: &mut Core, suns: Vec<SunTrigger>, conditions: Vec<Condition>| {
        c.create_repeating_reminder_in(&l, "x", daily_at(&["09:00"]), suns, conditions, None, 0)
    };
    assert!(matches!(
        make_with(&mut c, vec![], vec![days(&[])]),
        Err(Error::BadCondition(_))
    ));
    assert!(matches!(
        make_with(&mut c, vec![], vec![window("10:00", "10:00")]),
        Err(Error::BadCondition(_))
    ));
    assert!(matches!(
        make_with(&mut c, vec![sun(SunEvent::Sunset, 13 * 60)], vec![]),
        Err(Error::BadSunEvent(_))
    ));
    assert!(matches!(
        c.create_repeating_reminder_in(&l, "x", vec![], vec![], vec![], None, 0),
        Err(Error::BadSchedule(_))
    ));
    assert!(matches!(c.set_home(91.0, 0.0, 0), Err(Error::BadHome(_))));
    assert!(matches!(c.set_home(0.0, 181.0, 0), Err(Error::BadHome(_))));
    assert!(matches!(
        c.set_home(f64::NAN, 0.0, 0),
        Err(Error::BadHome(_))
    ));
    assert_eq!(c.home(), None);
    // Conditions don't apply to one-offs or countdowns.
    let once = c.create_reminder("Once", 1_000, 0).unwrap();
    let err = c
        .edit_reminder(
            &once,
            EditReminder {
                conditions: Some(vec![days(&["MO"])]),
                ..Default::default()
            },
            0,
        )
        .unwrap_err();
    assert!(matches!(err, Error::BadCondition(_)), "{err}");
    let counting = c
        .create_countdown_reminder(
            "Plants",
            hab_core::Countdown {
                amount: 3,
                unit: hab_core::CountdownUnit::Days,
                at: None,
            },
            None,
            Some(0),
            0,
        )
        .unwrap();
    for edit in [
        EditReminder {
            conditions: Some(vec![days(&["MO"])]),
            ..Default::default()
        },
        EditReminder {
            suns: Some(vec![sun(SunEvent::Sunset, 0)]),
            ..Default::default()
        },
    ] {
        assert!(c.edit_reminder(&counting, edit, 0).is_err());
    }
}

// ---- Pause, alerter, inbox ----

#[test]
fn a_pause_skips_the_instants_that_pass_the_conditions_and_records_nothing_for_the_others() {
    let mut c = core();
    let start = at("2026-10-02T06:00:00"); // Friday
    let rid = make(&mut c, &["09:00"], vec![days(&["FR", "SA"])], start);
    c.pause_reminder(&rid, Some(at("2026-10-10T00:00:00")), start)
        .unwrap();
    fires(&mut c, start, at("2026-10-09T12:00:00"), 6 * HOUR);
    let skipped: Vec<i64> = c
        .state()
        .occurrences
        .values()
        .map(|o| o.scheduled_at)
        .collect();
    // Friday and Saturday this week were skipped by the pause; Sunday to
    // Thursday passed the conditions and left no record at all; next Friday
    // (the 9th) is in the pause too.
    assert_eq!(
        skipped,
        [
            at("2026-10-02T09:00:00"),
            at("2026-10-03T09:00:00"),
            at("2026-10-09T09:00:00")
        ]
    );
    assert!(c.state().occurrences.values().all(|o| o.closing.is_some()));
}

#[test]
fn the_alerter_and_the_inbox_see_only_what_fired() {
    let mut c = core();
    let start = at("2026-10-02T06:00:00"); // Friday
    make(&mut c, &["09:00"], vec![days(&["MO"])], start);
    let mut alerter = Alerter::new();
    // Friday and Saturday 09:00: nothing fires, nothing is alerted.
    for t in [at("2026-10-02T09:00:00"), at("2026-10-03T09:00:00")] {
        c.tick(t).unwrap();
        assert!(alerter.pass(&mut c, t, false).unwrap().commands.is_empty());
        let inbox = c.inbox(t);
        assert!(inbox.overdue.is_empty() && inbox.due.is_empty());
    }
    // Monday it fires and alerts like any occurrence.
    let monday = at("2026-10-05T09:00:00");
    assert_eq!(c.tick(monday).unwrap().len(), 1);
    let pass = alerter.pass(&mut c, monday, false).unwrap();
    assert!(pass
        .commands
        .iter()
        .any(|cmd| matches!(cmd, Command::Show(n) if n.title == "Water the garden")));
    assert_eq!(c.inbox(monday).due.len(), 1);
}

#[test]
fn a_sun_event_alerts_like_any_other_occurrence() {
    let mut c = core();
    let now = at("2024-06-20T00:00:00");
    c.set_home(LONDON.0, LONDON.1, now).unwrap();
    let l = list(&c);
    let rid = c
        .create_repeating_reminder_in(
            &l,
            "Close the chickens in",
            vec![],
            vec![sun(SunEvent::Sunset, 0)],
            vec![],
            Some("UTC"),
            now,
        )
        .unwrap();
    c.edit_reminder(
        &rid,
        EditReminder {
            priority: Some(Priority::High),
            ..Default::default()
        },
        now,
    )
    .unwrap();
    let t = c.next_fire_at().unwrap();
    let mut alerter = Alerter::new();
    assert!(alerter
        .pass(&mut c, t - 1, false)
        .unwrap()
        .commands
        .is_empty());
    c.tick(t).unwrap();
    let pass = alerter.pass(&mut c, t, false).unwrap();
    assert!(pass
        .commands
        .iter()
        .any(|cmd| matches!(cmd, Command::Show(n) if n.title == "Close the chickens in")));
}

// ---- Formats ----

#[test]
fn sun_events_conditions_and_the_home_are_format_eleven_and_a_plain_reminder_is_not() {
    assert_eq!(FORMAT_VERSION, 11);
    let plain = Event::RecurringReminderCreated {
        reminder_id: "r".into(),
        title: "t".into(),
        schedules: vec![],
        zone: None,
        suns: vec![],
        conditions: vec![],
    };
    assert_eq!(plain.format(), 2);
    // An older app would fire it on every day and ignore sun events.
    let with_conditions = Event::RecurringReminderCreated {
        reminder_id: "r".into(),
        title: "t".into(),
        schedules: vec![],
        zone: None,
        suns: vec![],
        conditions: vec![Condition::Daylight],
    };
    assert_eq!(with_conditions.format(), 11);
    let with_sun = Event::RecurringReminderCreated {
        reminder_id: "r".into(),
        title: "t".into(),
        schedules: vec![],
        zone: None,
        suns: vec![sun(SunEvent::Sunset, 0)],
        conditions: vec![],
    };
    assert_eq!(with_sun.format(), 11);
    for change in [
        Change::SunEvents(vec![]),
        Change::Conditions(vec![days(&["MO"])]),
    ] {
        assert_eq!(
            Event::ReminderEdited {
                reminder_id: "r".into(),
                hlc: Hlc::default(),
                change
            }
            .format(),
            11
        );
    }
    assert_eq!(
        Event::HomeSet {
            hlc: Hlc::default(),
            place: Some(Place::home(1.0, 2.0).unwrap())
        }
        .format(),
        11
    );
    // The creation event of a plain reminder has the same JSON as before,
    // so a format-2 reader reads it.
    let json = serde_json::to_value(&plain).unwrap();
    assert!(json.get("suns").is_none() && json.get("conditions").is_none());
    let old: Event = serde_json::from_value(json).unwrap();
    assert_eq!(old, plain);
}

#[test]
fn what_a_standalone_device_stores_for_a_conditional_reminder_is_format_eleven() {
    let mut c = core();
    c.join(USER, "1").unwrap();
    make(&mut c, &["09:00"], vec![days(&["MO"])], 0);
    c.set_home(1.0, 2.0, 0).unwrap();
    let formats: Vec<u32> = c.unsent().unwrap().iter().map(|o| o.format).collect();
    assert_eq!(formats, [11, 11]);
}

// ---- Two devices ----

/// Delivers what `from` made to `to`, numbering as a server would.
fn sync(from: &mut Core, to: &mut Core, device: &str, first_seq: i64, list: &str) -> i64 {
    let out = from.unsent().unwrap();
    let n = out.len() as i64;
    for (i, o) in out.into_iter().enumerate() {
        let seq = first_seq + i as i64;
        to.receive(list, seq, &o.event_id, device, o.format, &o.payload)
            .unwrap();
        from.mark_sent(&o.event_id, seq).unwrap();
    }
    n
}

#[test]
fn two_devices_agree_on_the_home_the_predictions_and_the_firings() {
    let mut a = core();
    a.join(USER, "1").unwrap();
    let list = a.personal_list_id().to_string();
    let mut b = core();
    b.join(USER, "2").unwrap();
    b.use_personal_list(&list).unwrap();
    let now = at("2024-06-20T00:00:00");
    // The home, a sun reminder and a conditional one are made on A.
    a.set_home(LONDON.0, LONDON.1, now).unwrap();
    let l = list.clone();
    a.create_repeating_reminder_in(
        &l,
        "Sunset",
        vec![],
        vec![sun(SunEvent::Sunset, -30)],
        vec![days(&["MO", "TU", "WE", "TH", "FR"]), Condition::Daylight],
        Some("Europe/London"),
        now,
    )
    .unwrap();
    let seq = sync(&mut a, &mut b, "1", 1, &list);
    assert_eq!(seq, 2);
    assert_eq!(b.home(), a.home());
    let until = at("2024-06-30T00:00:00");
    let ea = a.expected(now, until);
    let eb = b.expected(now, until);
    assert_eq!(ea.len(), 7); // the 20th, 21st and 24th to 28th
    assert_eq!(
        ea.iter().map(|e| e.scheduled_at).collect::<Vec<_>>(),
        eb.iter().map(|e| e.scheduled_at).collect::<Vec<_>>()
    );
    assert_eq!(a.next_fire_at(), b.next_fire_at());
    // Both fire the same occurrence, which merges into one.
    let t = a.next_fire_at().unwrap();
    let fa = a.tick(t).unwrap();
    let fb = b.tick(t + 5).unwrap();
    assert_eq!(fa[0].occurrence_id, fb[0].occurrence_id);
    sync(&mut a, &mut b, "1", 3, &list);
    let n = sync(&mut b, &mut a, "2", 100, &list);
    assert!(n >= 1);
    assert_eq!(a.state().occurrences.len(), 1);
    assert_eq!(b.state().occurrences.len(), 1);
}

#[test]
fn of_two_homes_set_out_of_touch_the_later_clock_wins_everywhere() {
    let mut a = core();
    a.join(USER, "1").unwrap();
    let list = a.personal_list_id().to_string();
    let mut b = core();
    b.join(USER, "2").unwrap();
    b.use_personal_list(&list).unwrap();
    a.set_home(10.0, 20.0, 1_000).unwrap();
    b.set_home(30.0, 40.0, 2_000).unwrap();
    sync(&mut a, &mut b, "1", 1, &list);
    sync(&mut b, &mut a, "2", 10, &list);
    assert_eq!(a.home(), b.home());
    assert_eq!(a.home().unwrap().latitude, 30.0);
    // Clearing on one device clears it on the other.
    a.clear_home(3_000).unwrap();
    sync(&mut a, &mut b, "1", 20, &list);
    assert_eq!(b.home(), None);
}
