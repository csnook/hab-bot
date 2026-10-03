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
