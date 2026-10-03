use super::*;

const T0: Millis = 1_700_000_000_000;

/// The open occurrences: the Inbox's Due section.
fn due(c: &Core) -> Vec<InboxItem> {
    c.inbox(T0)
        .into_iter()
        .filter(|i| i.section == InboxSection::Due)
        .collect()
}

fn core() -> Core {
    Core::open_in_memory("me").unwrap()
}

#[test]
fn a_reminder_does_not_fire_before_its_time() {
    let mut c = core();
    c.create_one_off("Call the plumber", T0 + 1000, T0).unwrap();
    assert!(c.fire_due(T0 + 999).unwrap().is_empty());
    assert!(due(&c).is_empty());
    assert_eq!(c.next_due(T0), Some(T0 + 1000));
}

#[test]
fn a_reminder_fires_at_its_time_and_lists_under_due() {
    let mut c = core();
    c.create_one_off("Call the plumber", T0 + 1000, T0).unwrap();
    let opened = c.fire_due(T0 + 1000).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].title, "Call the plumber");
    let inbox = due(&c);
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].section, InboxSection::Due);
    assert_eq!(c.next_due(T0), None);
}

#[test]
fn firing_twice_opens_one_occurrence() {
    let mut c = core();
    c.create_one_off("x", T0, T0).unwrap();
    assert_eq!(c.fire_due(T0).unwrap().len(), 1);
    assert!(c.fire_due(T0 + 5).unwrap().is_empty());
    assert_eq!(due(&c).len(), 1);
}

#[test]
fn completing_records_who_and_when_and_finishes_the_one_off() {
    let mut c = core();
    let rid = c.create_one_off("x", T0, T0).unwrap();
    let occ = c.fire_due(T0).unwrap().remove(0);
    c.complete(&occ.id, T0 + 50).unwrap();
    assert!(due(&c).is_empty());
    let done = c.state().occurrence(&occ.id).unwrap();
    assert_eq!(done.status, OccurrenceStatus::Completed);
    assert_eq!(done.completed_by.as_deref(), Some("me"));
    assert_eq!(done.closed_at, Some(T0 + 50));
    assert!(
        c.state()
            .reminders()
            .find(|r| r.id == rid)
            .unwrap()
            .finished
    );
    assert!(c.fire_due(T0 + 100).unwrap().is_empty());
}

#[test]
fn completing_twice_or_unknown_is_an_error() {
    let mut c = core();
    c.create_one_off("x", T0, T0).unwrap();
    let occ = c.fire_due(T0).unwrap().remove(0);
    c.complete(&occ.id, T0).unwrap();
    assert!(matches!(
        c.complete(&occ.id, T0),
        Err(Error::NoOpenOccurrence(_))
    ));
    assert!(matches!(
        c.complete("nope", T0),
        Err(Error::NoOpenOccurrence(_))
    ));
}

#[test]
fn empty_title_is_rejected() {
    assert!(matches!(
        core().create_one_off("  ", T0, T0),
        Err(Error::EmptyTitle)
    ));
}

#[test]
fn restarting_rebuilds_the_same_state_and_fires_what_passed_while_closed() {
    let path = std::env::temp_dir().join(format!("hab-core-{}.db", uuid::Uuid::new_v4()));
    let p = path.to_str().unwrap();
    {
        let mut c = Core::open(p, "me").unwrap();
        c.create_one_off("while closed", T0 + 1000, T0).unwrap();
        c.create_one_off("already done", T0, T0).unwrap();
        let occ = c.fire_due(T0).unwrap().remove(0);
        c.complete(&occ.id, T0 + 1).unwrap();
    }
    let mut c = Core::open(p, "me").unwrap();
    assert!(due(&c).is_empty());
    assert_eq!(c.next_due(T0), Some(T0 + 1000));
    // the app was closed past the time
    let opened = c.fire_due(T0 + 60_000).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].title, "while closed");
    drop(c);
    let c = Core::open(p, "me").unwrap();
    assert_eq!(due(&c).len(), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn refresh_sees_events_appended_by_another_core() {
    let path = std::env::temp_dir().join(format!("hab-core-{}.db", uuid::Uuid::new_v4()));
    let p = path.to_str().unwrap();
    let mut app = Core::open(p, "me").unwrap();
    let mut alarm = Core::open(p, "me").unwrap();
    app.create_one_off("x", T0, T0).unwrap();
    // the alarm path fires it with the app open
    alarm.refresh().unwrap();
    assert_eq!(alarm.fire_due(T0).unwrap().len(), 1);
    assert!(due(&app).is_empty());
    app.refresh().unwrap();
    assert_eq!(due(&app).len(), 1);
    // and completes it; the app's own firing afterwards is a no-op
    let id = due(&app)[0].occurrence.id.clone();
    alarm.complete(&id, T0 + 1).unwrap();
    app.refresh().unwrap();
    assert!(due(&app).is_empty());
    assert!(app.fire_due(T0 + 2).unwrap().is_empty());
    let _ = std::fs::remove_file(path);
}

mod time_tests {
    use crate::time::*;
    use chrono::NaiveDate;

    fn wall(y: i32, m: u32, d: u32, h: u32, min: u32) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    #[test]
    fn spring_forward_fires_at_three() {
        let tz = parse_zone("America/New_York").unwrap();
        // 2026-03-08: 2:00 jumps to 3:00
        let at = resolve(wall(2026, 3, 8, 2, 30), tz);
        assert_eq!(wall_at(at, tz), wall(2026, 3, 8, 3, 0));
    }

    #[test]
    fn fall_back_resolves_to_the_first_time() {
        let tz = parse_zone("America/New_York").unwrap();
        // 2026-11-01: 2:00 returns to 1:00, so 1:30 happens twice
        let first = resolve(wall(2026, 11, 1, 1, 30), tz);
        // 1:30 EDT is 05:30 UTC; the second 1:30 (EST) is 06:30 UTC
        assert_eq!(first, wall(2026, 11, 1, 5, 30).and_utc().timestamp_millis());
    }

    #[test]
    fn instances_follow_the_rule() {
        let start = wall(2026, 10, 5, 7, 0); // a Monday
        let got = instances(
            "FREQ=WEEKLY;BYDAY=MO,WE",
            start,
            wall(2026, 10, 5, 0, 0),
            wall(2026, 10, 14, 0, 0),
        )
        .unwrap();
        assert_eq!(
            got,
            vec![
                wall(2026, 10, 5, 7, 0),
                wall(2026, 10, 7, 7, 0),
                wall(2026, 10, 12, 7, 0)
            ]
        );
        let monthly = instances(
            "FREQ=MONTHLY;BYDAY=1MO",
            start,
            wall(2026, 11, 1, 0, 0),
            wall(2026, 12, 31, 0, 0),
        )
        .unwrap();
        assert_eq!(
            monthly,
            vec![wall(2026, 11, 2, 7, 0), wall(2026, 12, 7, 7, 0)]
        );
    }

    #[test]
    fn a_bad_rule_is_refused() {
        assert!(validate("FREQ=SOMETIMES", wall(2026, 1, 1, 7, 0)).is_err());
    }
}

mod schedules {
    use super::*;
    use crate::time::{parse_zone, resolve};
    use chrono::NaiveDate;

    fn wall(y: i32, m: u32, d: u32, h: u32, min: u32) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    fn at(zone: &str, y: i32, m: u32, d: u32, h: u32, min: u32) -> Millis {
        resolve(wall(y, m, d, h, min), parse_zone(zone).unwrap())
    }

    fn daily(
        core: &mut Core,
        title: &str,
        start: &str,
        created: Millis,
        tz: Option<&str>,
    ) -> String {
        core.create(
            NewReminder {
                title: title.into(),
                triggers: vec![Trigger::Schedule {
                    rule: "FREQ=DAILY".into(),
                    start: start.into(),
                }],
                tz: tz.map(String::from),
                priority: Priority::Medium,
                expiry: None,
            },
            created,
        )
        .unwrap()
    }

    const NY: &str = "America/New_York";

    #[test]
    fn a_schedule_fires_each_instance_and_misses_the_unanswered_one() {
        let mut c = core();
        c.set_zone("UTC");
        let created = at("UTC", 2026, 10, 5, 6, 0);
        let rid = daily(&mut c, "Vitamins", "2026-10-05T07:00", created, None);
        assert!(c
            .fire_due(at("UTC", 2026, 10, 5, 6, 59))
            .unwrap()
            .is_empty());
        let first = c.fire_due(at("UTC", 2026, 10, 5, 7, 0)).unwrap();
        assert_eq!(first.len(), 1);
        // not answered; the next day's instance fires and the old one is missed
        let second = c.fire_due(at("UTC", 2026, 10, 6, 7, 0)).unwrap();
        assert_eq!(second.len(), 1);
        let old = c.state().occurrence(&first[0].id).unwrap();
        assert_eq!(old.status, OccurrenceStatus::Missed);
        assert_eq!(old.closed_at, Some(at("UTC", 2026, 10, 6, 7, 0)));
        let open: Vec<_> = c
            .state()
            .occurrences()
            .filter(|o| o.status == OccurrenceStatus::Due)
            .collect();
        assert_eq!(open.len(), 1, "never more than one open occurrence");
        assert_eq!(open[0].reminder_id, rid);
        // the reminder itself goes on
        assert!(!c.state().reminder(&rid).unwrap().finished);
        c.complete(&second[0].id, at("UTC", 2026, 10, 6, 7, 5))
            .unwrap();
        assert!(!c.state().reminder(&rid).unwrap().finished);
    }

    #[test]
    fn an_instance_before_creation_does_not_fire() {
        let mut c = core();
        c.set_zone("UTC");
        // made at 9:00 for 7:00 daily: today's 7:00 is already past
        daily(
            &mut c,
            "x",
            "2026-10-05T07:00",
            at("UTC", 2026, 10, 5, 9, 0),
            None,
        );
        assert!(c.fire_due(at("UTC", 2026, 10, 5, 9, 1)).unwrap().is_empty());
        assert_eq!(c.fire_due(at("UTC", 2026, 10, 6, 7, 0)).unwrap().len(), 1);
    }

    #[test]
    fn firing_twice_for_the_same_instance_makes_one_occurrence() {
        let mut c = core();
        c.set_zone("UTC");
        daily(
            &mut c,
            "x",
            "2026-10-05T07:00",
            at("UTC", 2026, 10, 5, 6, 0),
            None,
        );
        let now = at("UTC", 2026, 10, 5, 7, 0);
        assert_eq!(c.fire_due(now).unwrap().len(), 1);
        assert!(c.fire_due(now + 1000).unwrap().is_empty());
        assert_eq!(c.state().occurrences().count(), 1);
    }

    #[test]
    fn waking_late_fires_the_latest_instance_and_records_the_rest_missed() {
        let mut c = core();
        c.set_zone("UTC");
        daily(
            &mut c,
            "x",
            "2026-10-05T07:00",
            at("UTC", 2026, 10, 5, 6, 0),
            None,
        );
        // asleep for three days
        let opened = c.fire_due(at("UTC", 2026, 10, 8, 12, 0)).unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(
            opened[0].scheduled_at,
            at("UTC", 2026, 10, 8, 7, 0),
            "counts from the scheduled time"
        );
        let all: Vec<_> = c.state().occurrences().collect();
        assert_eq!(all.len(), 4);
        assert_eq!(
            all.iter()
                .filter(|o| o.status == OccurrenceStatus::Missed)
                .count(),
            3
        );
        assert_eq!(
            all.iter()
                .filter(|o| o.status == OccurrenceStatus::Due)
                .count(),
            1
        );
    }

    #[test]
    fn several_times_a_day_are_several_triggers() {
        let mut c = core();
        c.set_zone("UTC");
        let rule = |start: &str| Trigger::Schedule {
            rule: "FREQ=DAILY".into(),
            start: start.into(),
        };
        c.create(
            NewReminder {
                title: "Pills".into(),
                triggers: vec![rule("2026-10-05T08:00"), rule("2026-10-05T20:00")],
                tz: None,
                priority: Priority::Medium,
                expiry: None,
            },
            at("UTC", 2026, 10, 5, 0, 0),
        )
        .unwrap();
        let morning = c.fire_due(at("UTC", 2026, 10, 5, 8, 0)).unwrap();
        let evening = c.fire_due(at("UTC", 2026, 10, 5, 20, 0)).unwrap();
        assert_eq!(evening.len(), 1);
        // the unfinished 8:00 occurrence is missed when the 20:00 one fires
        assert_eq!(
            c.state().occurrence(&morning[0].id).unwrap().status,
            OccurrenceStatus::Missed
        );
    }

    #[test]
    fn spring_forward_2_30_fires_at_3_00_once() {
        let mut c = core();
        c.set_zone(NY);
        daily(
            &mut c,
            "x",
            "2026-03-07T02:30",
            at(NY, 2026, 3, 7, 0, 0),
            Some(NY),
        );
        assert_eq!(c.fire_due(at(NY, 2026, 3, 7, 2, 30)).unwrap().len(), 1);
        assert!(c.fire_due(at(NY, 2026, 3, 8, 1, 59)).unwrap().is_empty());
        let fired = c.fire_due(at(NY, 2026, 3, 8, 3, 0)).unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].scheduled_at, at(NY, 2026, 3, 8, 3, 0));
    }

    #[test]
    fn fall_back_1_30_fires_once_the_first_time() {
        let mut c = core();
        c.set_zone(NY);
        daily(
            &mut c,
            "x",
            "2026-10-31T01:30",
            at(NY, 2026, 10, 31, 0, 0),
            Some(NY),
        );
        c.fire_due(at(NY, 2026, 10, 31, 1, 30)).unwrap();
        let first = at(NY, 2026, 11, 1, 1, 30);
        assert_eq!(c.fire_due(first).unwrap().len(), 1);
        // an hour later the clock shows 1:30 again: nothing fires
        assert!(c.fire_due(first + 3600 * 1000).unwrap().is_empty());
        assert!(c.fire_due(first + 3 * 3600 * 1000).unwrap().is_empty());
        assert_eq!(c.fire_due(at(NY, 2026, 11, 2, 1, 30)).unwrap().len(), 1);
    }

    #[test]
    fn a_floating_reminder_follows_the_device_zone_and_a_pinned_one_does_not() {
        let created = at("UTC", 2026, 10, 4, 0, 0);
        let mut floating = core();
        let mut pinned = core();
        floating.set_zone("Europe/London");
        daily(&mut floating, "f", "2026-10-05T07:00", created, None);
        daily(
            &mut pinned,
            "p",
            "2026-10-05T07:00",
            created,
            Some("Europe/London"),
        );
        // the device travels to Tokyo
        floating.set_zone("Asia/Tokyo");
        pinned.set_zone("Asia/Tokyo");
        // 07:00 in Tokyo is 22:00 UTC the day before; 07:00 in London (BST) is 06:00 UTC
        let tokyo_7 = at("Asia/Tokyo", 2026, 10, 5, 7, 0);
        let london_7 = at("Europe/London", 2026, 10, 5, 7, 0);
        assert!(tokyo_7 < london_7);
        assert_eq!(floating.fire_due(tokyo_7).unwrap().len(), 1);
        assert!(pinned.fire_due(tokyo_7).unwrap().is_empty());
        assert_eq!(pinned.fire_due(london_7).unwrap().len(), 1);
    }

    #[test]
    fn the_inbox_lists_expected_later_today_and_closed_earlier_today() {
        let mut c = core();
        c.set_zone("UTC");
        let rule = |start: &str| Trigger::Schedule {
            rule: "FREQ=DAILY".into(),
            start: start.into(),
        };
        c.create(
            NewReminder {
                title: "Pills".into(),
                triggers: vec![rule("2026-10-05T08:00"), rule("2026-10-05T20:00")],
                tz: None,
                priority: Priority::Medium,
                expiry: None,
            },
            at("UTC", 2026, 10, 5, 0, 0),
        )
        .unwrap();
        let noon = at("UTC", 2026, 10, 5, 12, 0);
        c.fire_due(at("UTC", 2026, 10, 5, 8, 0)).unwrap();
        let later: Vec<_> = c
            .inbox(noon)
            .into_iter()
            .filter(|i| i.section == InboxSection::LaterToday)
            .collect();
        assert_eq!(later.len(), 1);
        assert_eq!(
            later[0].occurrence.scheduled_at,
            at("UTC", 2026, 10, 5, 20, 0)
        );
        assert_eq!(later[0].occurrence.status, OccurrenceStatus::Expected);
        // the 20:00 instance fires and misses the 8:00 one, which lists under Earlier today
        c.fire_due(at("UTC", 2026, 10, 5, 20, 0)).unwrap();
        let evening = at("UTC", 2026, 10, 5, 21, 0);
        let earlier: Vec<_> = c
            .inbox(evening)
            .into_iter()
            .filter(|i| i.section == InboxSection::EarlierToday)
            .collect();
        assert_eq!(earlier.len(), 1);
        assert_eq!(earlier[0].occurrence.status, OccurrenceStatus::Missed);
    }

    #[test]
    fn bad_schedules_and_zones_are_refused() {
        let mut c = core();
        let make = |rule: &str, tz: Option<&str>| NewReminder {
            title: "x".into(),
            triggers: vec![Trigger::Schedule {
                rule: rule.into(),
                start: "2026-10-05T07:00".into(),
            }],
            tz: tz.map(String::from),
            priority: Priority::Medium,
            expiry: None,
        };
        assert!(c.create(make("FREQ=NEVER", None), 0).is_err());
        assert!(c.create(make("FREQ=DAILY", Some("Mars/Base")), 0).is_err());
        assert!(c
            .create(
                NewReminder {
                    title: "x".into(),
                    triggers: vec![],
                    tz: None,
                    priority: Priority::Medium,
                    expiry: None,
                },
                0
            )
            .is_err());
    }

    #[test]
    fn next_due_follows_the_schedule() {
        let mut c = core();
        c.set_zone("UTC");
        let now = at("UTC", 2026, 10, 5, 6, 0);
        daily(&mut c, "x", "2026-10-05T07:00", now, None);
        assert_eq!(c.next_due(now), Some(at("UTC", 2026, 10, 5, 7, 0)));
        c.fire_due(at("UTC", 2026, 10, 5, 7, 0)).unwrap();
        assert_eq!(
            c.next_due(at("UTC", 2026, 10, 5, 7, 0)),
            Some(at("UTC", 2026, 10, 6, 7, 0))
        );
    }
}

mod countdowns {
    use super::*;

    const HOUR: Millis = 3_600_000;
    const DAY: Millis = 24 * HOUR;

    fn countdown(
        core: &mut Core,
        unit: CountdownUnit,
        amount: u32,
        at: Option<&str>,
        last: Option<Millis>,
        now: Millis,
    ) -> String {
        core.create(
            NewReminder {
                title: "Water the plants".into(),
                triggers: vec![Trigger::Countdown {
                    unit,
                    amount,
                    at: at.map(String::from),
                    last_done: last,
                }],
                tz: None,
                priority: Priority::Medium,
                expiry: None,
            },
            now,
        )
        .unwrap()
    }

    #[test]
    fn counts_elapsed_time_from_when_it_was_last_done() {
        let mut c = core();
        let id = countdown(
            &mut c,
            CountdownUnit::Hours,
            8,
            None,
            Some(T0 - 2 * HOUR),
            T0,
        );
        assert_eq!(c.next_due(T0), Some(T0 + 6 * HOUR));
        assert!(c.fire_due(T0 + 6 * HOUR - 1).unwrap().is_empty());
        let fired = c.fire_due(T0 + 6 * HOUR).unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].reminder_id, id);
        // while open, it does not predict or fire again
        assert_eq!(c.next_due(T0 + 7 * HOUR), None);
        assert!(c.fire_due(T0 + 30 * HOUR).unwrap().is_empty());
        assert!(c.expected(T0 + 7 * HOUR, T0 + 99 * DAY).is_empty());
    }

    #[test]
    fn never_done_fires_at_once() {
        let mut c = core();
        countdown(&mut c, CountdownUnit::Days, 3, None, None, T0);
        assert_eq!(c.fire_due(T0).unwrap().len(), 1);
    }

    #[test]
    fn a_completion_restarts_it_from_the_recorded_time() {
        let mut c = core();
        countdown(&mut c, CountdownUnit::Hours, 8, None, None, T0);
        let occ = c.fire_due(T0).unwrap().remove(0);
        // done at 11:00 but recorded as 9:40 (an hour and twenty before)
        let tapped = T0 + 100 * 60_000;
        let recorded = T0 + 40 * 60_000;
        c.complete(&occ.id, recorded).unwrap();
        let _ = tapped;
        assert_eq!(c.next_due(tapped), Some(recorded + 8 * HOUR));
        assert!(c.fire_due(recorded + 8 * HOUR - 1).unwrap().is_empty());
        assert_eq!(c.fire_due(recorded + 8 * HOUR).unwrap().len(), 1);
    }

    #[test]
    fn a_miss_restarts_it_from_when_it_closed() {
        let mut c = core();
        countdown(&mut c, CountdownUnit::Hours, 8, None, None, T0);
        let occ = c.fire_due(T0).unwrap().remove(0);
        // the app closes it as missed (as expiry will)
        c.record(Event::OccurrenceMissed {
            occurrence_id: occ.id,
            at: T0 + 20 * HOUR,
        })
        .unwrap();
        assert_eq!(c.next_due(T0 + 20 * HOUR), Some(T0 + 28 * HOUR));
    }

    #[test]
    fn completing_early_restarts_it_and_cancels_the_pending_firing() {
        let mut c = core();
        let id = countdown(&mut c, CountdownUnit::Hours, 8, None, Some(T0), T0);
        let first_due = T0 + 8 * HOUR;
        assert_eq!(c.next_due(T0), Some(first_due));
        // done 2 hours in, ahead of it
        c.complete_early(&id, T0 + 2 * HOUR, T0 + 2 * HOUR).unwrap();
        assert!(
            c.fire_due(first_due).unwrap().is_empty(),
            "the pending firing is cancelled"
        );
        assert_eq!(c.next_due(first_due), Some(T0 + 10 * HOUR));
        assert_eq!(c.fire_due(T0 + 10 * HOUR).unwrap().len(), 1);
    }

    #[test]
    fn it_predicts_only_its_next_occurrence() {
        let mut c = core();
        countdown(&mut c, CountdownUnit::Hours, 1, None, Some(T0), T0);
        assert_eq!(c.expected(T0, T0 + 30 * DAY).len(), 1);
    }

    #[test]
    fn days_can_fire_at_a_time_of_day_in_the_reminders_zone() {
        let mut c = core();
        c.set_zone("UTC");
        // last done 2026-10-05 12:00 UTC; 3 days later at 9:00
        let last = chrono::NaiveDate::from_ymd_opt(2026, 10, 5)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        countdown(
            &mut c,
            CountdownUnit::Days,
            3,
            Some("09:00"),
            Some(last),
            last,
        );
        let want = chrono::NaiveDate::from_ymd_opt(2026, 10, 8)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        assert_eq!(c.next_due(last), Some(want));
    }

    #[test]
    fn elapsed_countdowns_ignore_the_time_zone() {
        let mut a = core();
        let mut b = core();
        a.set_zone("Asia/Tokyo");
        b.set_zone("America/New_York");
        countdown(&mut a, CountdownUnit::Hours, 8, None, Some(T0), T0);
        countdown(&mut b, CountdownUnit::Hours, 8, None, Some(T0), T0);
        assert_eq!(a.next_due(T0), b.next_due(T0));
    }

    #[test]
    fn bad_countdowns_are_refused() {
        let mut c = core();
        let make = |unit, amount, at: Option<&str>| NewReminder {
            title: "x".into(),
            triggers: vec![Trigger::Countdown {
                unit,
                amount,
                at: at.map(String::from),
                last_done: None,
            }],
            tz: None,
            priority: Priority::Medium,
            expiry: None,
        };
        assert!(c.create(make(CountdownUnit::Days, 0, None), T0).is_err());
        assert!(c
            .create(make(CountdownUnit::Hours, 2, Some("09:00")), T0)
            .is_err());
        assert!(c
            .create(make(CountdownUnit::Days, 2, Some("25:99")), T0)
            .is_err());
    }
}

mod priorities {
    use super::*;

    const MINUTE: Millis = 60_000;
    const HOUR: Millis = 60 * MINUTE;
    const DAY: Millis = 24 * HOUR;

    fn one_off(c: &mut Core, title: &str, priority: Priority) -> String {
        c.create(
            NewReminder {
                title: title.into(),
                triggers: vec![Trigger::OneOff { at: T0 }],
                tz: None,
                priority,
                expiry: None,
            },
            T0,
        )
        .unwrap()
    }

    fn sections(c: &Core, now: Millis) -> Vec<(InboxSection, String)> {
        c.inbox(now)
            .into_iter()
            .filter(|i| matches!(i.section, InboxSection::Overdue | InboxSection::Due))
            .map(|i| (i.section, i.occurrence.title))
            .collect()
    }

    #[test]
    fn the_five_built_ins_carry_the_specs_table() {
        let all = all_settings();
        assert_eq!(
            all.iter().map(|s| s.name).collect::<Vec<_>>(),
            ["Minimum", "Low", "Medium", "High", "Maximum"]
        );
        let due: Vec<_> = all.iter().map(|s| s.due_interval).collect();
        assert_eq!(due, [DAY, DAY, HOUR, 0, 0]);
        let overdue: Vec<_> = all.iter().map(|s| s.overdue_interval).collect();
        assert_eq!(overdue, [DAY, DAY, 10 * MINUTE, 10 * MINUTE, 10 * MINUTE]);
        let med = Priority::Medium.settings();
        assert_eq!(med.due_style, AlertStyle::Gentle);
        assert_eq!(
            med.overdue,
            vec![
                Escalation {
                    after: 0,
                    style: AlertStyle::Insistent
                },
                Escalation {
                    after: HOUR,
                    style: AlertStyle::Alarm
                }
            ]
        );
        assert_eq!(
            all.iter().map(|s| s.swipeable).collect::<Vec<_>>(),
            [true, true, true, false, false]
        );
        assert_eq!(
            all.iter()
                .map(|s| s.breaks_do_not_disturb)
                .collect::<Vec<_>>(),
            [false, false, false, false, true]
        );
        assert_eq!(
            all.iter().map(|s| s.server_wait).collect::<Vec<_>>(),
            [Some(60_000), Some(60_000), Some(60_000), Some(60_000), None]
        );
        assert_eq!(Priority::Minimum.settings().due_style, AlertStyle::Silent);
        assert_eq!(Priority::Maximum.settings().due_style, AlertStyle::Alarm);
    }

    #[test]
    fn an_occurrence_goes_overdue_after_its_priority_s_due_interval() {
        for (priority, after) in [
            (Priority::Minimum, DAY),
            (Priority::Low, DAY),
            (Priority::Medium, HOUR),
            (Priority::High, 0),
            (Priority::Maximum, 0),
        ] {
            let mut c = core();
            one_off(&mut c, "x", priority);
            c.fire_due(T0).unwrap();
            let expect =
                |now, section| assert_eq!(sections(&c, now)[0].0, section, "{priority:?} at {now}");
            if after > 0 {
                expect(T0 + after - 1, InboxSection::Due);
            }
            expect(T0 + after, InboxSection::Overdue);
        }
    }

    #[test]
    fn overdue_time_counts_from_the_scheduled_time_not_the_late_firing() {
        let mut c = core();
        one_off(&mut c, "x", Priority::Medium);
        // the device woke 90 minutes late
        c.fire_due(T0 + 90 * MINUTE).unwrap();
        assert_eq!(sections(&c, T0 + 90 * MINUTE)[0].0, InboxSection::Overdue);
    }

    #[test]
    fn overdue_sorts_by_highest_priority_then_longest_overdue() {
        let mut c = core();
        let make = |c: &mut Core, title: &str, at: Millis, priority| {
            c.create(
                NewReminder {
                    title: title.into(),
                    triggers: vec![Trigger::OneOff { at }],
                    tz: None,
                    priority,
                    expiry: None,
                },
                T0,
            )
            .unwrap();
        };
        make(&mut c, "low, long overdue", T0 - 5 * DAY, Priority::Low);
        make(&mut c, "medium, recent", T0 - 2 * HOUR, Priority::Medium);
        make(&mut c, "medium, older", T0 - 4 * HOUR, Priority::Medium);
        make(&mut c, "high", T0 - HOUR, Priority::High);
        make(
            &mut c,
            "not yet overdue",
            T0 - 10 * MINUTE,
            Priority::Medium,
        );
        c.fire_due(T0).unwrap();
        let titles: Vec<_> = sections(&c, T0).into_iter().map(|(_, t)| t).collect();
        assert_eq!(
            titles,
            [
                "high",
                "medium, older",
                "medium, recent",
                "low, long overdue",
                "not yet overdue"
            ]
        );
        assert_eq!(sections(&c, T0)[4].0, InboxSection::Due);
    }

    #[test]
    fn changing_priority_moves_the_overdue_time() {
        let mut c = core();
        let id = one_off(&mut c, "x", Priority::Low);
        c.fire_due(T0).unwrap();
        assert_eq!(sections(&c, T0 + 2 * HOUR)[0].0, InboxSection::Due);
        c.set_priority(&id, Priority::Medium).unwrap();
        assert_eq!(sections(&c, T0 + 2 * HOUR)[0].0, InboxSection::Overdue);
    }

    #[test]
    fn a_skeleton_era_event_gets_the_default_priority() {
        let old = r#"{"type":"reminder_created","reminder_id":"r","title":"t","due_at":5,"created_at":1}"#;
        let e: Event = serde_json::from_str(old).unwrap();
        let s = State::from_events([e]);
        assert_eq!(s.reminder("r").unwrap().priority, Priority::Medium);
    }
}

mod alert_tests {
    use super::*;

    const MINUTE: Millis = 60_000;
    const HOUR: Millis = 60 * MINUTE;
    const DAY: Millis = 24 * HOUR;

    fn setup(priority: Priority) -> (Core, AlertEngine) {
        let mut c = core();
        c.create(
            NewReminder {
                title: "Call".into(),
                triggers: vec![Trigger::OneOff { at: T0 }],
                tz: None,
                priority,
                expiry: None,
            },
            T0,
        )
        .unwrap();
        c.fire_due(T0).unwrap();
        (c, AlertEngine::new("laptop"))
    }

    fn styles(p: &Poll) -> Vec<(AlertKind, AlertStyle)> {
        p.alerts.iter().map(|a| (a.kind, a.style)).collect()
    }

    #[test]
    fn gentle_alerts_once_and_does_not_repeat() {
        let (mut c, mut e) = setup(Priority::Low);
        assert_eq!(
            styles(&e.poll(&mut c, T0, false).unwrap()),
            [(AlertKind::First, AlertStyle::Gentle)]
        );
        assert!(e.poll(&mut c, T0 + HOUR, false).unwrap().alerts.is_empty());
        // overdue after a day: Low goes silent, in the overdue list
        let p = e.poll(&mut c, T0 + DAY, false).unwrap();
        assert_eq!(styles(&p), [(AlertKind::StyleChange, AlertStyle::Silent)]);
        assert!(p.alerts[0].overdue);
        assert!(e
            .poll(&mut c, T0 + 5 * DAY, false)
            .unwrap()
            .alerts
            .is_empty());
    }

    #[test]
    fn minimum_is_silent_throughout() {
        let (mut c, mut e) = setup(Priority::Minimum);
        assert_eq!(
            styles(&e.poll(&mut c, T0, false).unwrap()),
            [(AlertKind::First, AlertStyle::Silent)]
        );
        assert!(e
            .poll(&mut c, T0 + 2 * DAY, false)
            .unwrap()
            .alerts
            .is_empty());
    }

    #[test]
    fn medium_goes_insistent_then_alarm_and_repeats_every_interval() {
        let (mut c, mut e) = setup(Priority::Medium);
        assert_eq!(
            styles(&e.poll(&mut c, T0, false).unwrap()),
            [(AlertKind::First, AlertStyle::Gentle)]
        );
        // due for an hour, then overdue: insistent
        assert_eq!(
            styles(&e.poll(&mut c, T0 + HOUR, false).unwrap()),
            [(AlertKind::StyleChange, AlertStyle::Insistent)]
        );
        assert!(e
            .poll(&mut c, T0 + HOUR + 9 * MINUTE, false)
            .unwrap()
            .alerts
            .is_empty());
        assert_eq!(
            styles(&e.poll(&mut c, T0 + HOUR + 10 * MINUTE, false).unwrap()),
            [(AlertKind::Repeat, AlertStyle::Insistent)]
        );
        assert_eq!(
            styles(&e.poll(&mut c, T0 + HOUR + 20 * MINUTE, false).unwrap()),
            [(AlertKind::Repeat, AlertStyle::Insistent)]
        );
        // an hour overdue: the alarm, which keeps escalating until it closes
        assert_eq!(
            styles(&e.poll(&mut c, T0 + 2 * HOUR, false).unwrap()),
            [(AlertKind::StyleChange, AlertStyle::Alarm)]
        );
        assert_eq!(
            styles(&e.poll(&mut c, T0 + 2 * HOUR + 10 * MINUTE, false).unwrap()),
            [(AlertKind::Repeat, AlertStyle::Alarm)]
        );
    }

    #[test]
    fn high_alarms_at_once_and_repeats() {
        let (mut c, mut e) = setup(Priority::High);
        assert_eq!(
            styles(&e.poll(&mut c, T0, false).unwrap()),
            [(AlertKind::First, AlertStyle::Alarm)]
        );
        assert_eq!(
            styles(&e.poll(&mut c, T0 + 10 * MINUTE, false).unwrap()),
            [(AlertKind::Repeat, AlertStyle::Alarm)]
        );
    }

    #[test]
    fn closing_dismisses_the_notification_and_stops_alerts() {
        let (mut c, mut e) = setup(Priority::Medium);
        let id = e.poll(&mut c, T0, false).unwrap().alerts[0]
            .occurrence_id
            .clone();
        c.complete(&id, T0 + 1).unwrap();
        let p = e.poll(&mut c, T0 + 2 * HOUR, false).unwrap();
        assert!(p.alerts.is_empty());
        assert_eq!(p.dismissed, [id]);
        assert!(e
            .poll(&mut c, T0 + 3 * HOUR, false)
            .unwrap()
            .dismissed
            .is_empty());
    }

    #[test]
    fn do_not_disturb_downgrades_to_silent_and_catches_up_afterwards() {
        let (mut c, mut e) = setup(Priority::Medium);
        assert_eq!(
            styles(&e.poll(&mut c, T0, true).unwrap()),
            [(AlertKind::First, AlertStyle::Silent)]
        );
        // overdue and escalated while it was on: still silent, nothing repeats
        assert!(e
            .poll(&mut c, T0 + HOUR + 30 * MINUTE, true)
            .unwrap()
            .alerts
            .is_empty());
        // it ends: the alert arrives at its current level
        assert_eq!(
            styles(&e.poll(&mut c, T0 + HOUR + 40 * MINUTE, false).unwrap()),
            [(AlertKind::StyleChange, AlertStyle::Insistent)]
        );
    }

    #[test]
    fn maximum_breaks_do_not_disturb() {
        let (mut c, mut e) = setup(Priority::Maximum);
        assert_eq!(
            styles(&e.poll(&mut c, T0, true).unwrap()),
            [(AlertKind::First, AlertStyle::Alarm)]
        );
        let (mut c, mut e) = setup(Priority::High);
        assert_eq!(
            styles(&e.poll(&mut c, T0, true).unwrap()),
            [(AlertKind::First, AlertStyle::Silent)]
        );
    }

    fn alert_history(c: &Core) -> Vec<(String, AlertStyle, String)> {
        c.history()
            .into_iter()
            .filter_map(|e| match e {
                Event::AlertChanged {
                    occurrence_id,
                    style,
                    device,
                    ..
                } => Some((occurrence_id, style, device)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn history_records_the_first_alert_and_style_changes_with_the_device_but_not_repeats() {
        let (mut c, mut e) = setup(Priority::Medium);
        e.poll(&mut c, T0, false).unwrap(); // first: gentle
        e.poll(&mut c, T0 + HOUR, false).unwrap(); // insistent
        e.poll(&mut c, T0 + HOUR + 10 * MINUTE, false).unwrap(); // repeat
        e.poll(&mut c, T0 + HOUR + 20 * MINUTE, false).unwrap(); // repeat
        e.poll(&mut c, T0 + 2 * HOUR, false).unwrap(); // alarm
        let h = alert_history(&c);
        assert_eq!(
            h.iter()
                .map(|(_, s, d)| (*s, d.as_str()))
                .collect::<Vec<_>>(),
            [
                (AlertStyle::Gentle, "laptop"),
                (AlertStyle::Insistent, "laptop"),
                (AlertStyle::Alarm, "laptop")
            ]
        );
    }

    #[test]
    fn a_do_not_disturb_downgrade_is_not_a_change_of_style_in_history() {
        let (mut c, mut e) = setup(Priority::Low);
        e.poll(&mut c, T0, true).unwrap();
        e.poll(&mut c, T0 + MINUTE, false).unwrap();
        assert_eq!(alert_history(&c).len(), 1);
    }

    #[test]
    fn next_wake_finds_the_next_thing_to_do() {
        let (mut c, mut e) = setup(Priority::Medium);
        assert_eq!(e.next_wake(&c, T0), Some(T0), "the first alert is pending");
        e.poll(&mut c, T0, false).unwrap();
        assert_eq!(e.next_wake(&c, T0), Some(T0 + HOUR), "it goes overdue");
        e.poll(&mut c, T0 + HOUR, false).unwrap();
        assert_eq!(
            e.next_wake(&c, T0 + HOUR),
            Some(T0 + HOUR + 10 * MINUTE),
            "the repeat"
        );
    }

    #[test]
    fn skipping_closes_without_doing_it() {
        let (mut c, _) = setup(Priority::Medium);
        let id = c.open_occurrences()[0].id.clone();
        c.skip(&id, Some("away".into()), T0 + 5).unwrap();
        let o = c.state().occurrence(&id).unwrap();
        assert_eq!(o.status, OccurrenceStatus::Skipped);
        assert!(matches!(
            c.skip(&id, None, T0 + 6),
            Err(Error::NoOpenOccurrence(_))
        ));
    }
}

mod snooze_tests {
    use super::*;

    const MINUTE: Millis = 60_000;
    const HOUR: Millis = 60 * MINUTE;
    const DAY: Millis = 24 * HOUR;

    fn setup(priority: Priority, expiry: Option<Millis>) -> (Core, AlertEngine, String) {
        let mut c = core();
        c.create(
            NewReminder {
                title: "Call the plumber".into(),
                triggers: vec![Trigger::OneOff { at: T0 }],
                tz: None,
                priority,
                expiry,
            },
            T0,
        )
        .unwrap();
        let id = c.fire_due(T0).unwrap().remove(0).id;
        (c, AlertEngine::new("laptop"), id)
    }

    #[test]
    fn one_tap_uses_the_priority_s_current_interval() {
        for (priority, now, want) in [
            (Priority::Minimum, T0, DAY),
            (Priority::Low, T0, DAY),
            (Priority::Medium, T0, HOUR),                   // due
            (Priority::Medium, T0 + 2 * HOUR, 10 * MINUTE), // overdue
            (Priority::High, T0, 10 * MINUTE),
            (Priority::Maximum, T0, 10 * MINUTE),
        ] {
            let (mut c, _, id) = setup(priority, None);
            let until = c.snooze_default(&id, SnoozeVia::Button, now).unwrap();
            assert_eq!(until, now + want, "{priority:?}");
            assert_eq!(
                c.state().occurrence(&id).unwrap().snoozed_until,
                Some(until)
            );
        }
    }

    #[test]
    fn the_picker_offers_interval_one_hour_and_tomorrow_morning() {
        let (c, _, id) = setup(Priority::Low, None);
        let p = c.snooze_picker(&id, T0).unwrap();
        let labels: Vec<_> = p.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, ["Default", "1 hour", "Tomorrow morning"]);
        assert_eq!(p.options[0].until, T0 + DAY);
        assert_eq!(p.options[1].until, T0 + HOUR);
        assert!(p.options[2].until > T0 + HOUR);
        assert_eq!(p.expires_at, None);
        // Medium's interval is already an hour: no duplicate
        let (c, _, id) = setup(Priority::Medium, None);
        let labels: Vec<_> = c
            .snooze_picker(&id, T0)
            .unwrap()
            .options
            .into_iter()
            .map(|o| o.label)
            .collect();
        assert_eq!(labels, ["Default", "Tomorrow morning"]);
    }

    #[test]
    fn the_picker_shows_a_known_expiry() {
        let (c, _, id) = setup(Priority::Medium, Some(5 * HOUR));
        assert_eq!(
            c.snooze_picker(&id, T0).unwrap().expires_at,
            Some(T0 + 5 * HOUR)
        );
    }

    #[test]
    fn a_snoozed_occurrence_stays_open_and_quiet_then_alerts_at_its_current_level() {
        let (mut c, mut e, id) = setup(Priority::Medium, None);
        e.poll(&mut c, T0, false).unwrap(); // gentle
        c.snooze(&id, T0 + 3 * HOUR, SnoozeVia::Button, T0 + MINUTE)
            .unwrap();
        // it goes overdue on schedule, quietly; the alarm step comes and goes unheard
        let p = e.poll(&mut c, T0 + HOUR + 30 * MINUTE, false).unwrap();
        assert!(p.alerts.iter().all(|a| a.style == AlertStyle::Silent));
        assert!(c.open_occurrences()[0].overdue_at <= T0 + HOUR + 30 * MINUTE);
        assert!(c.state().is_open(&id));
        // the snooze ends: alerts at its current level, the alarm
        let p = e.poll(&mut c, T0 + 3 * HOUR, false).unwrap();
        assert_eq!(p.alerts.len(), 1);
        assert_eq!(p.alerts[0].style, AlertStyle::Alarm);
    }

    #[test]
    fn each_snooze_is_recorded_with_what_it_was_set_to_end_on_and_how_it_ended() {
        let (mut c, _, id) = setup(Priority::Medium, None);
        c.snooze(&id, T0 + HOUR, SnoozeVia::Button, T0 + 1).unwrap();
        // snoozed again before it ended; repeats are unlimited
        c.snooze(&id, T0 + 2 * HOUR, SnoozeVia::Swipe, T0 + 2)
            .unwrap();
        c.fire_due(T0 + 2 * HOUR).unwrap(); // the second one runs out
        c.snooze(&id, T0 + 3 * HOUR, SnoozeVia::Button, T0 + 2 * HOUR + 1)
            .unwrap();
        c.complete(&id, T0 + 2 * HOUR + 2).unwrap(); // and the third is ended by closing
        let record: Vec<_> = c
            .history()
            .into_iter()
            .filter_map(|e| match e {
                Event::Snoozed { until, via, .. } => {
                    Some(format!("set until {} via {via:?}", until - T0))
                }
                Event::SnoozeEnded { how, at, .. } => Some(format!("ended {how:?} at {}", at - T0)),
                _ => None,
            })
            .collect();
        assert_eq!(
            record,
            [
                format!("set until {} via Button", HOUR),
                format!("ended Replaced at {}", 2),
                format!("set until {} via Swipe", 2 * HOUR),
                format!("ended Elapsed at {}", 2 * HOUR),
                format!("set until {} via Button", 3 * HOUR),
                format!("ended Closed at {}", 2 * HOUR + 2),
            ]
        );
        assert_eq!(c.state().occurrence(&id).unwrap().snoozed_until, None);
    }

    #[test]
    fn a_snooze_must_end_in_the_future() {
        let (mut c, _, id) = setup(Priority::Medium, None);
        assert!(c.snooze(&id, T0, SnoozeVia::Button, T0).is_err());
        assert!(c.snooze("nope", T0 + HOUR, SnoozeVia::Button, T0).is_err());
    }

    #[test]
    fn a_known_expiry_inside_the_snooze_gets_a_last_chance_alert_10_minutes_before() {
        let (mut c, mut e, id) = setup(Priority::Low, Some(2 * HOUR));
        e.poll(&mut c, T0, false).unwrap();
        c.snooze(&id, T0 + DAY, SnoozeVia::Button, T0 + MINUTE)
            .unwrap();
        // snoozing quiets the notification already showing
        let quiet = e.poll(&mut c, T0 + MINUTE, false).unwrap();
        assert_eq!(
            quiet.alerts.iter().map(|a| a.style).collect::<Vec<_>>(),
            [AlertStyle::Silent]
        );
        assert!(e
            .poll(&mut c, T0 + 2 * HOUR - 11 * MINUTE, false)
            .unwrap()
            .alerts
            .is_empty());
        let p = e.poll(&mut c, T0 + 2 * HOUR - 10 * MINUTE, false).unwrap();
        assert_eq!(p.alerts.len(), 1);
        let a = &p.alerts[0];
        assert_eq!(
            (a.kind, a.expires_at),
            (AlertKind::LastChance, Some(T0 + 2 * HOUR))
        );
        // Low's due style is gentle; never quieter than that
        assert_eq!(a.style, AlertStyle::Gentle);
        // once only
        assert!(e
            .poll(&mut c, T0 + 2 * HOUR - 5 * MINUTE, false)
            .unwrap()
            .alerts
            .is_empty());
        // then the expiry applies: missed at the expiry
        c.fire_due(T0 + 2 * HOUR).unwrap();
        let o = c.state().occurrence(&id).unwrap();
        assert_eq!(
            (o.status, o.closed_at),
            (OccurrenceStatus::Missed, Some(T0 + 2 * HOUR))
        );
    }

    #[test]
    fn last_chance_is_never_quieter_than_gentle_even_for_minimum() {
        let (mut c, mut e, id) = setup(Priority::Minimum, Some(HOUR));
        e.poll(&mut c, T0, false).unwrap();
        c.snooze(&id, T0 + DAY, SnoozeVia::Button, T0 + 1).unwrap();
        let p = e.poll(&mut c, T0 + 50 * MINUTE, false).unwrap();
        assert_eq!(p.alerts[0].style, AlertStyle::Gentle);
    }

    #[test]
    fn a_snooze_that_ends_before_the_expiry_needs_no_last_chance() {
        let (mut c, mut e, id) = setup(Priority::Low, Some(5 * HOUR));
        e.poll(&mut c, T0, false).unwrap();
        c.snooze(&id, T0 + HOUR, SnoozeVia::Button, T0 + 1).unwrap();
        let kinds: Vec<_> = (0..=6)
            .flat_map(|h| e.poll(&mut c, T0 + h * HOUR, false).unwrap().alerts)
            .map(|a| a.kind)
            .collect();
        assert!(!kinds.contains(&AlertKind::LastChance));
    }

    #[test]
    fn an_expected_occurrence_can_be_snoozed_ahead_of_time() {
        let mut c = core();
        c.set_zone("UTC");
        let start = T0 - (T0 % DAY) + DAY + 7 * HOUR; // 7:00 UTC the next day
        let rid = c
            .create(
                NewReminder {
                    title: "Medicine".into(),
                    triggers: vec![Trigger::Schedule {
                        rule: "FREQ=DAILY".into(),
                        start: crate::time::format_wall(crate::time::wall_at(
                            start,
                            chrono_tz::UTC,
                        )),
                    }],
                    tz: None,
                    priority: Priority::Medium,
                    expiry: Some(4 * HOUR),
                },
                T0,
            )
            .unwrap();
        let expected = c.expected(T0, T0 + 2 * DAY).remove(0);
        assert_eq!(expected.reminder_id, rid);
        // "make the 7:00 medicine 7:30"
        c.snooze(&expected.id, start + 30 * MINUTE, SnoozeVia::Button, T0)
            .unwrap();
        assert_eq!(
            c.expected(T0, T0 + 2 * DAY)[0].snoozed_until,
            Some(start + 30 * MINUTE)
        );
        let mut e = AlertEngine::new("laptop");
        // it fires at 7:00, quietly, into the lists
        let fired = c.fire_due(start).unwrap();
        assert_eq!(fired[0].snoozed_until, Some(start + 30 * MINUTE));
        let p = e.poll(&mut c, start, false).unwrap();
        assert_eq!(p.alerts[0].style, AlertStyle::Silent);
        // and alerts at 7:30
        let p = e.poll(&mut c, start + 30 * MINUTE, false).unwrap();
        assert_eq!(p.alerts[0].style, AlertStyle::Gentle);
        // its expiry still counts from the scheduled time
        assert_eq!(c.open_occurrences()[0].expires_at, Some(start + 4 * HOUR));
    }
}
