//! What the reminder editor needs of the core: overdue times and expiries that
//! are the next time a schedule matches, several expiries at once, the note on
//! occurrences, and a reminder as the editor shows it.

use hab_core::{
    Alerter, Change, ClosingKind, Command, Core, Delay, DelaySpec, EditReminder, Error, Event,
    Pattern, Priority, Schedule, Setting, TriggerView,
};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

const NY: &str = "America/New_York";
const HOUR: i64 = 3_600;

fn at(local: &str) -> i64 {
    let dt: DateTime = local.parse().unwrap();
    TimeZone::get(NY)
        .unwrap()
        .to_zoned(dt)
        .unwrap()
        .timestamp()
        .as_second()
}

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone(NY).unwrap();
    c
}

fn next(pattern: Pattern, time: &str) -> Delay {
    DelaySpec::Next {
        pattern,
        time: time.into(),
    }
    .to_delay()
    .unwrap()
}

fn monthly_first() -> Delay {
    next(Pattern::MonthlyByDate { day: 1 }, "00:00")
}

fn daily(time: &str) -> Delay {
    next(Pattern::Daily, time)
}

/// A one-off reminder firing at `fire`, with the given overdue and expiries.
fn one_off(c: &mut Core, fire: &str, overdue: Option<Delay>, expiry: Vec<Delay>) -> String {
    let t = at(fire);
    let id = c.create_reminder("Bins", t, t - 100).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            overdue: overdue.map(Some),
            expiry: Some(expiry),
            ..Default::default()
        },
        t - 100,
    )
    .unwrap();
    id
}

#[test]
fn overdue_can_be_the_next_time_a_schedule_matches() {
    let mut c = core();
    one_off(&mut c, "2026-10-05T09:00:00", Some(monthly_first()), vec![]);
    c.tick(at("2026-10-05T09:00:00")).unwrap();
    let first_of_november = at("2026-11-01T00:00:00");
    let due = c.snapshot().due;
    assert_eq!(due[0].overdue_at, first_of_november);
    // Not overdue by the priority's hour: the override holds.
    let later = at("2026-10-20T12:00:00");
    assert_eq!(c.inbox(later).due.len(), 1);
    assert!(c.inbox(later).overdue.is_empty());
    assert_eq!(c.next_overdue_at(later), Some(first_of_november));
    assert_eq!(c.inbox(first_of_november).overdue.len(), 1);
}

#[test]
fn the_next_time_is_strictly_after_the_scheduled_time() {
    let mut c = core();
    // Scheduled exactly on a 1st at 00:00: the next one is a month on.
    one_off(&mut c, "2026-10-01T00:00:00", Some(monthly_first()), vec![]);
    c.tick(at("2026-10-01T00:00:00")).unwrap();
    assert_eq!(c.snapshot().due[0].overdue_at, at("2026-11-01T00:00:00"));
}

#[test]
fn a_schedule_override_reads_in_the_reminders_own_zone() {
    let mut c = core();
    let id = c
        .create_recurring_reminder(
            "Call",
            vec![Schedule {
                start: "2026-10-05T09:00:00".into(),
                rule: "FREQ=DAILY".into(),
            }],
            Some("Asia/Tokyo"),
            at("2026-10-05T00:00:00"),
        )
        .unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            expiry: Some(vec![daily("23:59")]),
            ..Default::default()
        },
        at("2026-10-05T00:00:00"),
    )
    .unwrap();
    let tokyo = TimeZone::get("Asia/Tokyo").unwrap();
    let fire = tokyo
        .to_zoned("2026-10-06T09:00:00".parse::<DateTime>().unwrap())
        .unwrap()
        .timestamp()
        .as_second();
    c.tick(fire).unwrap();
    let expect = tokyo
        .to_zoned("2026-10-06T23:59:00".parse::<DateTime>().unwrap())
        .unwrap()
        .timestamp()
        .as_second();
    assert_eq!(c.snapshot().due[0].expires_at, Some(expect));
}

#[test]
fn clearing_the_override_goes_back_to_following_the_priority() {
    let mut c = core();
    let id = one_off(&mut c, "2026-10-05T09:00:00", Some(monthly_first()), vec![]);
    c.edit_reminder(
        &id,
        EditReminder {
            priority: Some(Priority::High),
            overdue: Some(None),
            ..Default::default()
        },
        at("2026-10-05T08:59:00"),
    )
    .unwrap();
    c.tick(at("2026-10-05T09:00:00")).unwrap();
    let high = Priority::High.settings().due_interval;
    assert_eq!(
        c.snapshot().due[0].overdue_at,
        at("2026-10-05T09:00:00") + high
    );
    let view = c.reminder_view(&id).unwrap();
    assert_eq!(view.overdue, None);
    assert_eq!(view.default_overdue_seconds, high);
}

#[test]
fn an_expiry_can_be_the_next_time_a_schedule_matches() {
    let mut c = core();
    one_off(&mut c, "2026-10-05T09:00:00", None, vec![daily("23:59")]);
    c.tick(at("2026-10-05T09:00:00")).unwrap();
    assert_eq!(c.next_fire_at(), Some(at("2026-10-05T23:59:00")));
    c.tick(at("2026-10-05T23:58:59")).unwrap();
    assert_eq!(c.snapshot().due.len(), 1);
    c.tick(at("2026-10-06T08:00:00")).unwrap();
    assert!(c.snapshot().due.is_empty());
    let o = c.state().occurrences.values().next().unwrap();
    let closing = o.closing.as_ref().unwrap();
    assert_eq!(closing.kind, ClosingKind::Missed);
    assert_eq!(closing.at, at("2026-10-05T23:59:00"));
}

#[test]
fn whichever_expiry_comes_first_marks_the_occurrence_missed() {
    // A delay that comes before the schedule.
    let mut c = core();
    one_off(
        &mut c,
        "2026-10-05T09:00:00",
        None,
        vec![daily("23:59"), Delay::After(5 * HOUR)],
    );
    c.tick(at("2026-10-05T09:00:00")).unwrap();
    assert_eq!(
        c.snapshot().due[0].expires_at,
        Some(at("2026-10-05T14:00:00"))
    );
    c.tick(at("2026-10-05T14:00:00")).unwrap();
    assert_eq!(
        c.state()
            .occurrences
            .values()
            .next()
            .unwrap()
            .closing
            .as_ref()
            .unwrap()
            .at,
        at("2026-10-05T14:00:00")
    );

    // And a schedule that comes before the delay.
    let mut c = core();
    one_off(
        &mut c,
        "2026-10-05T09:00:00",
        None,
        vec![Delay::After(20 * HOUR), daily("23:59")],
    );
    c.tick(at("2026-10-05T09:00:00")).unwrap();
    assert_eq!(
        c.snapshot().due[0].expires_at,
        Some(at("2026-10-05T23:59:00"))
    );
}

#[test]
fn a_late_firing_counts_expiry_from_the_scheduled_time() {
    let mut c = core();
    one_off(&mut c, "2026-10-05T09:00:00", None, vec![daily("23:59")]);
    // Asleep until the next morning: missed at once, nobody alerted.
    let fired = c.tick(at("2026-10-06T07:00:00")).unwrap();
    assert!(fired.is_empty());
    let o = c.state().occurrences.values().next().unwrap();
    assert_eq!(o.closing.as_ref().unwrap().kind, ClosingKind::Missed);
    assert_eq!(o.closing.as_ref().unwrap().at, at("2026-10-05T23:59:00"));
}

#[test]
fn expiry_delays_produce_no_expected_occurrences() {
    let from = at("2026-10-05T00:00:00");
    let until = at("2026-10-12T00:00:00");
    let make = |expiry: Vec<Delay>| {
        let mut c = core();
        let id = c
            .create_recurring_reminder(
                "Bins",
                vec![Schedule {
                    start: "2026-10-05T09:00:00".into(),
                    rule: "FREQ=DAILY".into(),
                }],
                None,
                from,
            )
            .unwrap();
        c.edit_reminder(
            &id,
            EditReminder {
                expiry: Some(expiry),
                overdue: Some(Some(monthly_first())),
                ..Default::default()
            },
            from,
        )
        .unwrap();
        c.expected(from, until)
    };
    let plain: Vec<i64> = make(vec![]).iter().map(|e| e.scheduled_at).collect();
    let with: Vec<i64> = make(vec![daily("23:59"), Delay::After(HOUR)])
        .iter()
        .map(|e| e.scheduled_at)
        .collect();
    assert_eq!(plain.len(), 7);
    assert_eq!(plain, with, "the delays add no expected occurrences");
    // A one-off has exactly its own.
    let mut c = core();
    one_off(&mut c, "2026-10-06T09:00:00", None, vec![daily("23:59")]);
    assert_eq!(c.expected(from, until).len(), 1);
}

#[test]
fn delays_are_validated() {
    let mut c = core();
    let id = one_off(&mut c, "2026-10-05T09:00:00", None, vec![]);
    let bad = Delay::Next(Schedule {
        start: "2026-10-05T09:00:00".into(),
        rule: "FREQ=SOMETIMES".into(),
    });
    let r = c.edit_reminder(
        &id,
        EditReminder {
            expiry: Some(vec![bad.clone()]),
            ..Default::default()
        },
        1,
    );
    assert!(matches!(r, Err(Error::BadDelay(_))));
    let r = c.edit_reminder(
        &id,
        EditReminder {
            overdue: Some(Some(bad)),
            ..Default::default()
        },
        1,
    );
    assert!(matches!(r, Err(Error::BadDelay(_))));
    let r = c.edit_reminder(
        &id,
        EditReminder {
            expiry: Some(vec![Delay::After(-5)]),
            ..Default::default()
        },
        1,
    );
    assert!(matches!(r, Err(Error::BadDuration)));
    assert!(DelaySpec::Other { rule: "x".into() }.to_delay().is_err());
    assert!(DelaySpec::Next {
        pattern: Pattern::Weekly { days: vec![] },
        time: "09:00".into()
    }
    .to_delay()
    .is_err());
}

#[test]
fn only_what_old_apps_cannot_read_needs_format_7() {
    let edit = |change| Event::ReminderEdited {
        reminder_id: "r".into(),
        hlc: Default::default(),
        change,
    };
    assert_eq!(edit(Change::Overdue(Some(Delay::After(5)))).format(), 3);
    assert_eq!(edit(Change::Overdue(None)).format(), 3);
    assert_eq!(edit(Change::Expiry(vec![])).format(), 3);
    assert_eq!(edit(Change::Expiry(vec![Delay::After(5)])).format(), 3);
    assert_eq!(edit(Change::Overdue(Some(monthly_first()))).format(), 7);
    assert_eq!(edit(Change::Expiry(vec![daily("23:59")])).format(), 7);
    assert_eq!(
        edit(Change::Expiry(vec![Delay::After(5), Delay::After(9)])).format(),
        7
    );
}

#[test]
fn the_old_wire_forms_still_read_and_the_new_ones_round_trip() {
    let json = |c: &Change| serde_json::to_value(c).unwrap();
    // What format 3 wrote.
    assert_eq!(
        json(&Change::Expiry(vec![Delay::After(60)])),
        serde_json::json!({"setting": "expiry", "value": 60})
    );
    assert_eq!(
        json(&Change::Expiry(vec![])),
        serde_json::json!({"setting": "expiry", "value": null})
    );
    assert_eq!(
        json(&Change::Overdue(Some(Delay::After(60)))),
        serde_json::json!({"setting": "overdue", "value": 60})
    );
    for old in [
        r#"{"setting":"expiry","value":60}"#,
        r#"{"setting":"expiry","value":null}"#,
        r#"{"setting":"overdue","value":null}"#,
        r#"{"setting":"overdue","value":30}"#,
    ] {
        serde_json::from_str::<Change>(old).unwrap();
    }
    let changes = [
        Change::Expiry(vec![Delay::After(60), daily("23:59")]),
        Change::Expiry(vec![daily("23:59")]),
        Change::Overdue(Some(monthly_first())),
    ];
    for c in changes {
        let back: Change = serde_json::from_value(json(&c)).unwrap();
        assert_eq!(back, c);
    }
}

#[test]
fn expiries_and_overdue_are_settings_with_history() {
    let mut c = core();
    let id = one_off(&mut c, "2026-10-05T09:00:00", None, vec![daily("23:59")]);
    c.edit_reminder(
        &id,
        EditReminder {
            expiry: Some(vec![Delay::After(HOUR)]),
            ..Default::default()
        },
        10,
    )
    .unwrap();
    let list = c.personal_list_id().to_string();
    let history = c.state_of(&list).unwrap().history(&id, Setting::Expiry);
    assert_eq!(history.len(), 2, "the schedule, then the hour");
    let old = history
        .iter()
        .find(|v| v.change == Change::Expiry(vec![daily("23:59")]))
        .unwrap()
        .event_id
        .clone();
    c.restore_setting(&id, Setting::Expiry, &old, 20).unwrap();
    assert_eq!(
        c.reminder_view(&id).unwrap().expiries,
        vec![DelaySpec::Next {
            pattern: Pattern::Daily,
            time: "23:59".into()
        }]
    );
}

#[test]
fn the_editor_reads_a_reminder_back() {
    let mut c = core();
    let t = at("2026-10-05T09:00:00");
    let id = c
        .create_recurring_reminder(
            "Bins",
            vec![Schedule::from_pattern(&Pattern::Weekdays, "2026-10-05", "07:30").unwrap()],
            Some(NY),
            t - 100,
        )
        .unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            note: Some("Green bin too".into()),
            priority: Some(Priority::Low),
            overdue: Some(Some(Delay::After(2 * HOUR))),
            expiry: Some(vec![Delay::After(HOUR), monthly_first()]),
            ..Default::default()
        },
        t - 100,
    )
    .unwrap();
    let v = c.reminder_view(&id).unwrap();
    assert_eq!(v.title, "Bins");
    assert_eq!(v.note, "Green bin too");
    assert_eq!(v.priority, Priority::Low);
    assert_eq!(v.zone.as_deref(), Some(NY));
    assert_eq!(v.list_name, None);
    assert_eq!(v.overdue, Some(DelaySpec::After { seconds: 7200 }));
    assert_eq!(
        v.default_overdue_seconds,
        Priority::Low.settings().due_interval
    );
    assert_eq!(
        v.expiries,
        vec![
            DelaySpec::After { seconds: 3600 },
            DelaySpec::Next {
                pattern: Pattern::MonthlyByDate { day: 1 },
                time: "00:00".into()
            }
        ]
    );
    let TriggerView::Schedules { schedules } = v.trigger else {
        panic!("a schedule reminder")
    };
    let parts = schedules[0].parts.as_ref().unwrap();
    assert_eq!(parts.pattern, Pattern::Weekdays);
    assert_eq!(
        (parts.date.as_str(), parts.time.as_str()),
        ("2026-10-05", "07:30")
    );

    let one = c.create_reminder("Once", t, t).unwrap();
    assert!(matches!(
        c.reminder_view(&one).unwrap().trigger,
        TriggerView::OneOff { fire_at } if fire_at == t
    ));
    assert!(c.reminder_view("nope").is_err());
}

#[test]
fn schedule_patterns_read_back_from_their_rules() {
    let cases = [
        Pattern::Daily,
        Pattern::Weekdays,
        Pattern::Weekly {
            days: vec!["MO".into(), "TH".into()],
        },
        Pattern::MonthlyByDate { day: 15 },
        Pattern::MonthlyByDate { day: -1 },
        Pattern::MonthlyByWeekday {
            ordinal: 2,
            weekday: "TU".into(),
        },
        Pattern::MonthlyByWeekday {
            ordinal: -1,
            weekday: "FR".into(),
        },
    ];
    for p in cases {
        let s = Schedule::from_pattern(&p, "2026-10-03", "09:30").unwrap();
        let parts = s.parts().unwrap_or_else(|| panic!("{p:?} reads back"));
        assert_eq!(parts.pattern, p);
        assert_eq!(parts.date, "2026-10-03");
        assert_eq!(parts.time, "09:30");
    }
    // Written by hand: not the editor's.
    for rule in [
        "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO",
        "FREQ=DAILY;COUNT=3",
        "FREQ=YEARLY",
        "FREQ=MONTHLY;BYDAY=5MO",
    ] {
        let s = Schedule {
            start: "2026-10-03T09:30:00".into(),
            rule: rule.into(),
        };
        assert!(s.parts().is_none(), "{rule}");
    }
    let seconds = Schedule {
        start: "2026-10-03T09:30:15".into(),
        rule: "FREQ=DAILY".into(),
    };
    assert!(seconds.parts().is_none());
    assert!(matches!(
        Delay::Next(Schedule {
            start: "2000-01-03T09:00:00".into(),
            rule: "FREQ=YEARLY".into()
        })
        .spec(),
        DelaySpec::Other { .. }
    ));
}

#[test]
fn the_note_is_on_every_view_of_the_occurrence() {
    let mut c = core();
    let t = at("2026-10-05T09:00:00");
    let id = c.create_reminder("Bins", t, t - 100).unwrap();
    let later = c.create_reminder("Later", t + 3 * HOUR, t - 100).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            note: Some("Green bin too".into()),
            ..Default::default()
        },
        t - 100,
    )
    .unwrap();
    c.edit_reminder(
        &later,
        EditReminder {
            note: Some("Ask about the key".into()),
            ..Default::default()
        },
        t - 100,
    )
    .unwrap();
    assert_eq!(
        c.snapshot()
            .upcoming
            .iter()
            .find(|u| u.reminder_id == id)
            .unwrap()
            .note,
        "Green bin too"
    );
    assert_eq!(
        c.expected(t - 200, t + 4 * HOUR)
            .iter()
            .find(|e| e.reminder_id == later)
            .unwrap()
            .note,
        "Ask about the key"
    );
    c.tick(t).unwrap();
    let due = &c.snapshot().due[0];
    assert_eq!(due.note, "Green bin too");
    assert_eq!(due.reminder_id, id);
    assert_eq!(
        c.occurrence_view(&due.occurrence_id).unwrap().note,
        "Green bin too"
    );
    // The notification carries it under "Due".
    let mut alerter = Alerter::new();
    let pass = alerter.pass(&mut c, t, false).unwrap();
    let shown = pass
        .commands
        .iter()
        .find_map(|c| match c {
            Command::Show(n) => Some(n.clone()),
            _ => None,
        })
        .expect("an alert");
    assert_eq!(shown.body, "Due\nGreen bin too");
    // A reminder without a note keeps the plain body.
    let mut c = core();
    c.create_reminder("Plain", t, t - 100).unwrap();
    c.tick(t).unwrap();
    let mut alerter = Alerter::new();
    let pass = alerter.pass(&mut c, t, false).unwrap();
    let Command::Show(n) = &pass.commands[0] else {
        panic!("an alert")
    };
    assert_eq!(n.body, "Due");
}
