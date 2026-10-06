//! Snoozing: the menu's choices, snoozing ahead of time, the last-chance
//! alert and the record of each snooze, through the core's own API.

use hab_core::{
    AlertStyle, Alerter, Command, Core, Countdown, CountdownUnit, Delay, EditReminder, Error,
    Event, Notification, Priority, SnoozeEnd, SnoozeKind, FORMAT_VERSION,
};

const MIN: i64 = 60;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
/// 2026-09-21 14:13:20 UTC.
const T0: i64 = 1_790_000_000;

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

fn edit(c: &mut Core, id: &str, p: Priority, expiry: Option<i64>) {
    c.edit_reminder(
        id,
        EditReminder {
            priority: Some(p),
            expiry: expiry.map(|d| vec![Delay::After(d)]),
            ..Default::default()
        },
        T0 - 2 * HOUR,
    )
    .unwrap();
}

/// A one-off reminder that fires at T0 with the occurrence open.
fn fired(c: &mut Core, title: &str, p: Priority, expiry: Option<i64>) -> (String, String) {
    let id = c.create_reminder(title, T0, T0 - 2 * HOUR).unwrap();
    edit(c, &id, p, expiry);
    c.tick(T0).unwrap();
    let occ = format!("{id}@{T0}");
    (id, occ)
}

fn shown(commands: &[Command]) -> Vec<&Notification> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::Show(n) => Some(n),
            _ => None,
        })
        .collect()
}

#[test]
fn the_menu_offers_the_priority_interval_an_hour_and_tomorrow_morning() {
    let tomorrow_8 = T0 - T0 % DAY + DAY + 8 * HOUR;
    let mut c = core();
    let table = [
        (Priority::Minimum, DAY, true),
        (Priority::Low, DAY, true),
        (Priority::Medium, HOUR, false),
        (Priority::High, 10 * MIN, true),
        (Priority::Maximum, 10 * MIN, true),
    ];
    for (p, length, plus_hour) in table {
        let (_, occ) = fired(&mut c, &format!("{p:?}"), p, None);
        let menu = c.snooze_picker(&occ, T0 + MIN).unwrap();
        let kinds: Vec<_> = menu.options.iter().map(|o| o.kind).collect();
        assert_eq!(
            kinds,
            if plus_hour {
                vec![
                    SnoozeKind::Interval,
                    SnoozeKind::Hour,
                    SnoozeKind::TomorrowMorning,
                ]
            } else {
                vec![SnoozeKind::Interval, SnoozeKind::TomorrowMorning]
            },
            "{p:?}"
        );
        assert_eq!(menu.options[0].until, T0 + MIN + length, "{p:?}");
        assert_eq!(menu.options.last().unwrap().until, tomorrow_8);
        assert_eq!(menu.expires_at, None);
    }
}

#[test]
fn the_medium_interval_is_ten_minutes_once_overdue() {
    let mut c = core();
    let (_, occ) = fired(&mut c, "Pills", Priority::Medium, None);
    let due = c.snooze_picker(&occ, T0 + MIN).unwrap();
    assert_eq!(due.options[0].seconds, Some(HOUR));
    let overdue = c.snooze_picker(&occ, T0 + HOUR + MIN).unwrap();
    assert_eq!(overdue.options[0].seconds, Some(10 * MIN));
    assert_eq!(overdue.options[1].kind, SnoozeKind::Hour);
}

#[test]
fn a_snooze_must_end_in_the_future() {
    let mut c = core();
    let (id, occ) = fired(&mut c, "Pills", Priority::Medium, None);
    assert!(matches!(
        c.snooze(&occ, T0 + MIN, T0 + MIN),
        Err(Error::SnoozeInThePast)
    ));
    assert!(matches!(
        c.snooze_expected(&id, T0 + DAY, T0, T0 + MIN),
        Err(Error::SnoozeInThePast)
    ));
}

#[test]
fn a_snoozed_occurrence_goes_overdue_on_schedule_quietly_then_alerts_at_its_level() {
    let mut c = core();
    let (_, occ) = fired(&mut c, "Pills", Priority::Medium, None);
    let mut a = Alerter::new();
    assert_eq!(shown(&a.pass(&mut c, T0, false).unwrap().commands).len(), 1);
    c.snooze(&occ, T0 + 2 * HOUR, T0 + MIN).unwrap();
    a.pass(&mut c, T0 + MIN, false).unwrap();
    // It goes overdue on schedule, counted from when it was scheduled...
    let overdue_at = T0 + HOUR;
    assert!(c.inbox(overdue_at - 1).overdue.is_empty());
    let inbox = c.inbox(overdue_at);
    assert_eq!(inbox.overdue.len(), 1);
    assert_eq!(inbox.overdue[0].snoozed_until, Some(T0 + 2 * HOUR));
    // ...without a sound.
    let p = a.pass(&mut c, overdue_at, false).unwrap();
    assert!(p.commands.is_empty());
    assert_eq!(p.next_at, Some(T0 + 2 * HOUR));
    // When the snooze ends it alerts at its current level.
    let p = a.pass(&mut c, T0 + 2 * HOUR, false).unwrap();
    // An hour overdue: the Medium escalation has reached the alarm.
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);
}

#[test]
fn repeated_snoozes_have_no_limit_and_each_is_recorded_with_how_it_ended() {
    let mut c = core();
    let (_, occ) = fired(&mut c, "Pills", Priority::Medium, None);
    // Ten snoozes in a row, each replacing the last.
    for i in 1..=10 {
        c.snooze(&occ, T0 + i * 20 * MIN, T0 + i * MIN).unwrap();
    }
    let h = c.snooze_history(&occ, T0 + 11 * MIN);
    assert_eq!(h.len(), 10);
    for (i, s) in h.iter().enumerate() {
        let i = i as i64 + 1;
        assert_eq!((s.set_at, s.until), (T0 + i * MIN, T0 + i * 20 * MIN));
        if i < 10 {
            // Replaced by the next one when that was made.
            assert_eq!(s.ended, Some(SnoozeEnd::Replaced));
            assert_eq!(s.ended_at, Some(T0 + (i + 1) * MIN));
        } else {
            assert_eq!((s.ended, s.ended_at), (None, None), "still holding");
        }
    }
    // The last runs out.
    let h = c.snooze_history(&occ, T0 + 10 * 20 * MIN);
    assert_eq!(h[9].ended, Some(SnoozeEnd::Elapsed));
    assert_eq!(h[9].ended_at, Some(T0 + 10 * 20 * MIN));
    // A snooze cut short by closing the occurrence.
    c.snooze(&occ, T0 + DAY, T0 + 4 * HOUR).unwrap();
    c.complete(&occ, T0 + 5 * HOUR).unwrap();
    let h = c.snooze_history(&occ, T0 + 6 * HOUR);
    assert_eq!(h[10].ended, Some(SnoozeEnd::Closed));
    assert_eq!(h[10].ended_at, Some(T0 + 5 * HOUR));
    assert_eq!(h[10].until, T0 + DAY);
}

#[test]
fn the_picker_shows_an_expiry_and_which_choices_it_falls_inside() {
    let mut c = core();
    let (_, occ) = fired(&mut c, "Plumber", Priority::Low, Some(3 * HOUR));
    let menu = c.snooze_picker(&occ, T0 + MIN).unwrap();
    assert_eq!(menu.expires_at, Some(T0 + 3 * HOUR));
    assert_eq!(menu.last_chance_at, Some(T0 + 3 * HOUR - 10 * MIN));
    // 1 day and tomorrow morning pass the expiry; so does the hour not.
    let flags: Vec<_> = menu
        .options
        .iter()
        .map(|o| (o.kind, o.last_chance))
        .collect();
    assert_eq!(
        flags,
        [
            (SnoozeKind::Interval, true),
            (SnoozeKind::Hour, false),
            (SnoozeKind::TomorrowMorning, true)
        ]
    );
}

#[test]
fn a_last_chance_alert_comes_ten_minutes_before_an_expiry_inside_the_snooze() {
    let mut c = core();
    let (_, occ) = fired(&mut c, "Plumber", Priority::Low, Some(3 * HOUR));
    let mut a = Alerter::new();
    a.pass(&mut c, T0, false).unwrap();
    c.snooze(&occ, T0 + DAY, T0 + MIN).unwrap();
    let chance = T0 + 3 * HOUR - 10 * MIN;
    let p = a.pass(&mut c, T0 + MIN, false).unwrap();
    assert_eq!(
        p.commands,
        [Command::Close {
            occurrence_id: occ.clone()
        }]
    );
    assert_eq!(p.next_at, Some(chance));
    assert!(a
        .pass(&mut c, chance - 1, false)
        .unwrap()
        .commands
        .is_empty());
    let p = a.pass(&mut c, chance, false).unwrap();
    let n = shown(&p.commands)[0].clone();
    assert_eq!(n.title, "Last chance: Plumber");
    // 14:13:20 + 3 hours, in UTC.
    assert_eq!(n.body, "Expires at 17:13");
    assert_eq!(n.style, AlertStyle::Gentle);
    // Once, and it stays up until the occurrence is acted on or closes.
    assert!(a
        .pass(&mut c, chance + MIN, false)
        .unwrap()
        .commands
        .is_empty());
    let styles: Vec<_> = c.state().alerts.iter().map(|x| x.style).collect();
    assert_eq!(styles.last(), Some(&AlertStyle::Gentle));
    // The expiry comes: missed, and the notification comes down.
    c.tick(T0 + 3 * HOUR).unwrap();
    let p = a.pass(&mut c, T0 + 3 * HOUR, false).unwrap();
    assert_eq!(p.commands, [Command::Close { occurrence_id: occ }]);
}

#[test]
fn the_last_chance_alert_is_the_due_style_but_never_quieter_than_gentle() {
    for (p, style) in [
        (Priority::Minimum, AlertStyle::Gentle),
        (Priority::Low, AlertStyle::Gentle),
        (Priority::Medium, AlertStyle::Gentle),
        (Priority::High, AlertStyle::Alarm),
        (Priority::Maximum, AlertStyle::Alarm),
    ] {
        let mut c = core();
        let (_, occ) = fired(&mut c, "Plumber", p, Some(3 * HOUR));
        c.snooze(&occ, T0 + DAY, T0 + MIN).unwrap();
        let mut a = Alerter::new();
        a.pass(&mut c, T0 + MIN, false).unwrap();
        let out = a.pass(&mut c, T0 + 3 * HOUR - 10 * MIN, false).unwrap();
        assert_eq!(shown(&out.commands)[0].style, style, "{p:?}");
    }
}

#[test]
fn no_last_chance_when_the_snooze_ends_first_or_the_expiry_is_not_known() {
    let mut c = core();
    // Ends well before the last-chance moment: it just alerts when it ends.
    let (_, occ) = fired(&mut c, "A", Priority::Low, Some(3 * HOUR));
    c.snooze(&occ, T0 + HOUR, T0 + MIN).unwrap();
    let mut a = Alerter::new();
    a.pass(&mut c, T0 + MIN, false).unwrap();
    let p = a.pass(&mut c, T0 + HOUR, false).unwrap();
    assert_eq!(shown(&p.commands)[0].title, "A");
    // No expiry: nothing to be last about.
    let mut c = core();
    let (_, occ) = fired(&mut c, "B", Priority::Low, None);
    c.snooze(&occ, T0 + DAY, T0 + MIN).unwrap();
    let mut a = Alerter::new();
    a.pass(&mut c, T0 + MIN, false).unwrap();
    assert!(a
        .pass(&mut c, T0 + 3 * HOUR, false)
        .unwrap()
        .commands
        .is_empty());
}

#[test]
fn a_snooze_made_inside_the_last_ten_minutes_holds_as_chosen() {
    let mut c = core();
    let (_, occ) = fired(&mut c, "A", Priority::Low, Some(3 * HOUR));
    let late = T0 + 3 * HOUR - 5 * MIN;
    c.snooze(&occ, T0 + DAY, late).unwrap();
    let mut a = Alerter::new();
    a.pass(&mut c, late, false).unwrap();
    assert!(a
        .pass(&mut c, late + MIN, false)
        .unwrap()
        .commands
        .is_empty());
}

#[test]
fn each_snooze_gets_its_own_last_chance_alert() {
    let mut c = core();
    let (_, occ) = fired(&mut c, "A", Priority::Low, Some(3 * HOUR));
    let mut a = Alerter::new();
    c.snooze(&occ, T0 + DAY, T0 + MIN).unwrap();
    a.pass(&mut c, T0 + MIN, false).unwrap();
    let chance = T0 + 3 * HOUR - 10 * MIN;
    assert_eq!(
        shown(&a.pass(&mut c, chance, false).unwrap().commands).len(),
        1
    );
    // Tapping Snooze on the last-chance alert takes it down. That snooze was
    // made inside the window, so it holds as chosen.
    c.snooze(&occ, T0 + DAY, chance + MIN).unwrap();
    let p = a.pass(&mut c, chance + MIN, false).unwrap();
    assert_eq!(p.commands, [Command::Close { occurrence_id: occ }]);
}

#[test]
fn an_expected_occurrence_snoozed_ahead_fires_quietly_and_alerts_when_the_snooze_ends() {
    let mut c = core();
    let id = c.create_reminder("Meds", T0, T0 - 2 * HOUR).unwrap();
    edit(&mut c, &id, Priority::Medium, Some(2 * HOUR));
    let now = T0 - HOUR;
    let menu = c.snooze_picker_expected(&id, T0, now).unwrap();
    // The choices count from the occurrence's time, which is when it fires.
    assert!(menu.ahead);
    assert_eq!(menu.options[0].until, T0 + HOUR);
    assert_eq!(menu.expires_at, Some(T0 + 2 * HOUR));
    let until = T0 + 30 * MIN;
    assert_eq!(
        c.snooze_expected(&id, T0, until, now).unwrap(),
        format!("{id}@{T0}")
    );
    let listed = c.expected(now, T0 + DAY);
    assert_eq!(listed[0].snoozed_until, Some(until));
    assert_eq!(listed[0].expires_at, Some(T0 + 2 * HOUR));
    // Nothing is open yet.
    assert!(c.state().occurrences.is_empty());

    let mut a = Alerter::new();
    assert!(a.pass(&mut c, now, false).unwrap().commands.is_empty());
    // It fires at its time, into the lists, quietly.
    let fired = c.tick(T0).unwrap();
    assert_eq!(fired.len(), 1);
    let inbox = c.inbox(T0);
    assert_eq!(inbox.due.len(), 1);
    assert_eq!(inbox.due[0].snoozed_until, Some(until));
    let p = a.pass(&mut c, T0, false).unwrap();
    assert!(p.commands.is_empty());
    assert_eq!(p.next_at, Some(until));
    // It alerts when the snooze ends; overdue and expiry count from T0.
    let p = a.pass(&mut c, until, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Gentle);
    assert_eq!(inbox.due[0].overdue_at, T0 + HOUR);
    assert_eq!(inbox.due[0].expires_at, Some(T0 + 2 * HOUR));
    c.tick(T0 + 2 * HOUR).unwrap();
    assert!(c.inbox(T0 + 2 * HOUR).due.is_empty());
    // And the history has it as made ahead of time.
    let h = c.snooze_history(&format!("{id}@{T0}"), T0 + 3 * HOUR);
    assert_eq!(h.len(), 1);
    assert!(h[0].ahead);
    assert_eq!(h[0].ended, Some(SnoozeEnd::Closed));
}

#[test]
fn a_snooze_ahead_can_be_replaced_and_a_closed_ahead_snooze_is_forgotten() {
    let mut c = core();
    let id = c.create_reminder("Meds", T0, T0 - 2 * HOUR).unwrap();
    let now = T0 - HOUR;
    c.snooze_expected(&id, T0, T0 + 10 * MIN, now).unwrap();
    c.snooze_expected(&id, T0, T0 + 20 * MIN, now + 1).unwrap();
    assert_eq!(
        c.expected(now, T0 + DAY)[0].snoozed_until,
        Some(T0 + 20 * MIN)
    );
    c.tick(T0).unwrap();
    assert_eq!(c.inbox(T0).due[0].snoozed_until, Some(T0 + 20 * MIN));
    let h = c.snooze_history(&format!("{id}@{T0}"), T0);
    assert_eq!(h[0].ended, Some(SnoozeEnd::Replaced));
    // Once it has fired, snoozing the expected occurrence snoozes the open one.
    c.snooze_expected(&id, T0, T0 + 40 * MIN, T0 + MIN).unwrap();
    assert_eq!(c.inbox(T0 + MIN).due[0].snoozed_until, Some(T0 + 40 * MIN));
}

#[test]
fn only_an_expected_occurrence_can_be_snoozed_ahead() {
    let mut c = core();
    let id = c.create_reminder("Meds", T0, T0 - 2 * HOUR).unwrap();
    // Not one of its times.
    assert!(matches!(
        c.snooze_expected(&id, T0 + 5, T0 + HOUR, T0 - HOUR),
        Err(Error::NotExpected(_))
    ));
    // Past, and closed.
    c.tick(T0).unwrap();
    c.complete(&format!("{id}@{T0}"), T0 + 1).unwrap();
    assert!(matches!(
        c.snooze_expected(&id, T0, T0 + HOUR, T0 + 2),
        Err(Error::NotExpected(_))
    ));
    assert!(c.snooze_expected("nope", T0, T0 + HOUR, T0 - HOUR).is_err());
}

#[test]
fn a_countdown_snoozed_ahead_fires_quietly_at_its_time() {
    let mut c = core();
    let id = c
        .create_countdown_reminder(
            "Plants",
            Countdown {
                amount: 3,
                unit: CountdownUnit::Hours,
                at: None,
            },
            None,
            Some(T0),
            T0,
        )
        .unwrap();
    let due = T0 + 3 * HOUR;
    c.snooze_expected(&id, due, due + HOUR, T0 + HOUR).unwrap();
    c.tick(due).unwrap();
    let inbox = c.inbox(due);
    assert_eq!(inbox.due[0].snoozed_until, Some(due + HOUR));
    assert_eq!(inbox.due[0].scheduled_at, due);
}

#[test]
fn snoozing_ahead_reaches_other_devices_before_or_after_the_occurrence_fires_there() {
    for fire_first in [false, true] {
        let mut a = Core::open_in_memory().unwrap();
        a.join("u1", "1").unwrap();
        let list = a.personal_list_id().to_string();
        let mut b = Core::open_in_memory().unwrap();
        b.join("u1", "2").unwrap();
        b.use_personal_list(&list).unwrap();
        let id = a.create_reminder("Meds", T0, T0 - 2 * HOUR).unwrap();
        let mut log: Vec<(String, String, u32, Vec<u8>)> = Vec::new();
        let upload = |c: &mut Core, dev: &str, log: &mut Vec<_>| {
            for o in c.unsent().unwrap() {
                log.push((
                    o.event_id.clone(),
                    dev.to_string(),
                    o.format,
                    o.payload.clone(),
                ));
                let seq = log.len() as i64;
                c.mark_sent(&o.event_id, seq).unwrap();
            }
        };
        let download = |c: &mut Core, log: &Vec<(String, String, u32, Vec<u8>)>| {
            for (n, (e, d, f, p)) in log.iter().enumerate() {
                c.receive(&list, n as i64 + 1, e, d, *f, p).unwrap();
            }
        };
        upload(&mut a, "1", &mut log);
        download(&mut b, &log);
        // Device 2 snoozes ahead; device 1 fires it.
        let until = T0 + 30 * MIN;
        b.snooze_expected(&id, T0, until, T0 - HOUR).unwrap();
        a.tick(T0).unwrap();
        if fire_first {
            upload(&mut a, "1", &mut log);
            upload(&mut b, "2", &mut log);
        } else {
            upload(&mut b, "2", &mut log);
            upload(&mut a, "1", &mut log);
        }
        download(&mut a, &log);
        download(&mut b, &log);
        for c in [&a, &b] {
            let due = c.inbox(T0).due;
            assert_eq!(due.len(), 1, "fire_first={fire_first}");
            assert_eq!(due[0].snoozed_until, Some(until), "fire_first={fire_first}");
        }
    }
}

#[test]
fn snoozing_ahead_is_a_format_6_event_an_older_app_keeps_without_applying() {
    assert_eq!(FORMAT_VERSION, 9);
    let e = Event::ExpectedOccurrenceSnoozed {
        reminder_id: "r".into(),
        scheduled_at: T0,
        until: T0 + 1,
    };
    assert_eq!(e.format(), 6);
    let mut c = core();
    c.join("u1", "1").unwrap();
    let id = c.create_reminder("Meds", T0, T0 - 2 * HOUR).unwrap();
    c.snooze_expected(&id, T0, T0 + 1, T0 - HOUR).unwrap();
    let formats: Vec<_> = c.unsent().unwrap().iter().map(|o| o.format).collect();
    assert_eq!(formats.last(), Some(&6));
    // Plain snoozes stay format 1.
    assert_eq!(
        Event::OccurrenceSnoozed {
            occurrence_id: "x".into(),
            until: 1
        }
        .format(),
        1
    );
}
