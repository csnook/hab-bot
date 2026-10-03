//! What a device alerts about and when, through the core's own API.

use hab_core::{
    AlertStyle, Alerter, Command, Core, EditReminder, Notification, Priority, Urgency, ACTION_DONE,
    ACTION_SKIP,
};

const MIN: i64 = 60;
const HOUR: i64 = 3_600;
const T0: i64 = 1_790_000_000;

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

/// A one-off reminder of `p` that fires at T0, with the occurrence open.
fn fired(c: &mut Core, title: &str, p: Priority) -> String {
    let id = c.create_reminder(title, T0, T0 - HOUR).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            priority: Some(p),
            ..Default::default()
        },
        T0 - HOUR,
    )
    .unwrap();
    c.tick(T0).unwrap();
    format!("{id}@{T0}")
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

fn history(c: &Core) -> Vec<AlertStyle> {
    c.state().alerts.iter().map(|a| a.style).collect()
}

#[test]
fn style_at_follows_the_due_style_then_the_largest_step_reached() {
    let s = Priority::Medium.settings();
    let overdue_at = T0 + HOUR;
    assert_eq!(s.style_at(overdue_at, T0), AlertStyle::Gentle);
    assert_eq!(s.style_at(overdue_at, overdue_at - 1), AlertStyle::Gentle);
    assert_eq!(s.style_at(overdue_at, overdue_at), AlertStyle::Insistent);
    assert_eq!(
        s.style_at(overdue_at, overdue_at + HOUR - 1),
        AlertStyle::Insistent
    );
    assert_eq!(s.style_at(overdue_at, overdue_at + HOUR), AlertStyle::Alarm);
    assert_eq!(
        s.style_at(overdue_at, overdue_at + 9 * HOUR),
        AlertStyle::Alarm
    );
    // Steps are counted from the overdue time, not the scheduled time.
    let high = Priority::High.settings();
    assert_eq!(high.style_at(T0, T0), AlertStyle::Alarm);
}

#[test]
fn silent_is_low_urgency_with_no_sound() {
    let mut c = core();
    fired(&mut c, "Water", Priority::Minimum);
    let pass = Alerter::new().pass(&mut c, T0, false).unwrap();
    let n = shown(&pass.commands)[0].clone();
    assert_eq!(n.style, AlertStyle::Silent);
    assert_eq!(n.urgency, Urgency::Low);
    assert_eq!(n.sound, None);
}

#[test]
fn gentle_is_normal_with_a_themed_sound_that_times_out() {
    let mut c = core();
    fired(&mut c, "Water", Priority::Low);
    let pass = Alerter::new().pass(&mut c, T0, false).unwrap();
    let n = shown(&pass.commands)[0].clone();
    assert_eq!(n.style, AlertStyle::Gentle);
    assert_eq!(n.urgency, Urgency::Normal);
    assert_eq!(n.sound, Some("message-new-instant"));
    assert!(n.timeout_ms > 0, "gentle times out");
    assert_eq!(n.title, "Water");
}

#[test]
fn notifications_carry_done_and_skip() {
    let mut c = core();
    fired(&mut c, "Water", Priority::Low);
    let pass = Alerter::new().pass(&mut c, T0, false).unwrap();
    let n = shown(&pass.commands)[0].clone();
    let keys: Vec<_> = n.actions.iter().map(|a| a.0).collect();
    assert_eq!(keys, [ACTION_DONE, ACTION_SKIP]);
}

#[test]
fn gentle_alerts_once_and_does_not_repeat() {
    let mut c = core();
    fired(&mut c, "Water", Priority::Low);
    let mut a = Alerter::new();
    assert_eq!(shown(&a.pass(&mut c, T0, false).unwrap().commands).len(), 1);
    for t in [T0 + 10 * MIN, T0 + HOUR, T0 + 5 * HOUR] {
        assert!(a.pass(&mut c, t, false).unwrap().commands.is_empty());
    }
}

#[test]
fn insistent_repeats_every_interval_until_closed_and_records_once() {
    let mut c = core();
    // Medium is Gentle for an hour, then Insistent every 10 minutes.
    let occ = fired(&mut c, "Pills", Priority::Medium);
    let mut a = Alerter::new();
    a.pass(&mut c, T0, false).unwrap();
    let overdue = T0 + HOUR;
    let first = a.pass(&mut c, overdue, false).unwrap();
    assert_eq!(shown(&first.commands)[0].style, AlertStyle::Insistent);
    assert_eq!(first.next_at, Some(overdue + 10 * MIN));
    // Nothing until the interval is up, then again and again.
    assert!(a
        .pass(&mut c, overdue + 10 * MIN - 1, false)
        .unwrap()
        .commands
        .is_empty());
    for k in 1..=5 {
        let p = a.pass(&mut c, overdue + k * 10 * MIN, false).unwrap();
        let n = shown(&p.commands);
        // Alarm joins after an hour overdue (#42); until then it still repeats.
        assert_eq!(n.len(), 1, "repeat {k}");
        assert_eq!(n[0].occurrence_id, occ);
    }
    // The repeats at the same style weren't recorded: Gentle, Insistent.
    assert_eq!(
        history(&c)[..2],
        [AlertStyle::Gentle, AlertStyle::Insistent]
    );
    // Closing ends it and takes the notification down.
    c.complete(&occ, overdue + 51 * MIN).unwrap();
    let p = a.pass(&mut c, overdue + 51 * MIN, false).unwrap();
    assert_eq!(
        p.commands,
        [Command::Close {
            occurrence_id: occ.clone()
        }]
    );
    assert!(a
        .pass(&mut c, overdue + 2 * HOUR, false)
        .unwrap()
        .commands
        .is_empty());
}

#[test]
fn escalation_follows_the_steps_and_each_change_is_recorded() {
    let mut c = core();
    let occ = fired(&mut c, "Pills", Priority::Medium);
    let mut a = Alerter::new();
    let overdue = T0 + HOUR;
    let styles: Vec<AlertStyle> = [T0, overdue, overdue + HOUR, overdue + 3 * HOUR]
        .iter()
        .map(|&t| {
            let p = a.pass(&mut c, t, false).unwrap();
            shown(&p.commands)[0].style
        })
        .collect();
    assert_eq!(
        styles,
        [
            AlertStyle::Gentle,
            AlertStyle::Insistent,
            AlertStyle::Alarm,
            AlertStyle::Alarm
        ]
    );
    // It keeps going until the occurrence closes.
    assert!(c.state().occurrences[&occ].is_open());
    // Alarm repeated at the last pass without a new history entry.
    assert_eq!(
        history(&c),
        [AlertStyle::Gentle, AlertStyle::Insistent, AlertStyle::Alarm]
    );
}

#[test]
fn the_next_wake_is_the_next_escalation_or_repeat() {
    let mut c = core();
    fired(&mut c, "Pills", Priority::Medium);
    let mut a = Alerter::new();
    let p = a.pass(&mut c, T0, false).unwrap();
    assert_eq!(p.next_at, Some(T0 + HOUR));
    let p = a.pass(&mut c, T0 + HOUR, false).unwrap();
    assert_eq!(p.next_at, Some(T0 + HOUR + 10 * MIN));
}

#[test]
fn history_records_the_user_and_the_device_that_alerted() {
    let mut c = core();
    c.join("u1", "dev-9").unwrap();
    fired(&mut c, "Water", Priority::Low);
    Alerter::new().pass(&mut c, T0, false).unwrap();
    let alerts = &c.state().alerts;
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].user, "u1");
    assert_eq!(alerts[0].device_id, "dev-9");
    assert_eq!(alerts[0].style, AlertStyle::Gentle);
}

#[test]
fn do_not_disturb_downgrades_to_silent_and_catches_up_afterwards() {
    let mut c = core();
    fired(&mut c, "Water", Priority::Low);
    let mut a = Alerter::new();
    let p = a.pass(&mut c, T0, true).unwrap();
    let n = shown(&p.commands)[0].clone();
    assert_eq!((n.style, n.sound), (AlertStyle::Silent, None));
    // Nothing more while it lasts.
    assert!(a.pass(&mut c, T0 + MIN, true).unwrap().commands.is_empty());
    // It ends: the occurrence alerts at its current level.
    let p = a.pass(&mut c, T0 + 2 * MIN, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Gentle);
    assert_eq!(history(&c), [AlertStyle::Silent, AlertStyle::Gentle]);
}

#[test]
fn a_priority_that_breaks_do_not_disturb_is_not_downgraded() {
    let mut c = core();
    fired(&mut c, "Meds", Priority::Maximum);
    fired(&mut c, "Bins", Priority::High);
    let p = Alerter::new().pass(&mut c, T0, true).unwrap();
    let mut by_title: Vec<(String, AlertStyle)> = shown(&p.commands)
        .iter()
        .map(|n| (n.title.clone(), n.style))
        .collect();
    by_title.sort();
    assert_eq!(
        by_title,
        [
            ("Bins".to_string(), AlertStyle::Silent),
            ("Meds".to_string(), AlertStyle::Alarm)
        ]
    );
}

#[test]
fn a_snooze_quiets_the_alert_and_ends_with_one_at_the_current_level() {
    let mut c = core();
    let occ = fired(&mut c, "Pills", Priority::Medium);
    let mut a = Alerter::new();
    a.pass(&mut c, T0, false).unwrap();
    c.snooze(&occ, T0 + 2 * HOUR, T0 + MIN).unwrap();
    // The notification comes down; overdue passes quietly.
    let p = a.pass(&mut c, T0 + MIN, false).unwrap();
    assert_eq!(
        p.commands,
        [Command::Close {
            occurrence_id: occ.clone()
        }]
    );
    assert!(p.next_at.is_some());
    assert!(a
        .pass(&mut c, T0 + HOUR + 5 * MIN, false)
        .unwrap()
        .commands
        .is_empty());
    // When the snooze ends it alerts at the level it has reached: alarm.
    let p = a.pass(&mut c, T0 + 2 * HOUR, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);
}

#[test]
fn an_acknowledged_occurrence_stops_alerting() {
    let mut c = core();
    let occ = fired(&mut c, "Pills", Priority::Medium);
    let mut a = Alerter::new();
    a.pass(&mut c, T0, false).unwrap();
    c.acknowledge(&occ, T0 + 1).unwrap();
    let p = a.pass(&mut c, T0 + HOUR, false).unwrap();
    assert!(shown(&p.commands).is_empty());
}

#[test]
fn a_restart_alerts_again_without_recording_the_same_style_twice() {
    let mut c = core();
    fired(&mut c, "Water", Priority::Low);
    Alerter::new().pass(&mut c, T0, false).unwrap();
    let p = Alerter::new().pass(&mut c, T0 + HOUR, false).unwrap();
    assert_eq!(shown(&p.commands).len(), 1);
    assert_eq!(history(&c), [AlertStyle::Gentle]);
}

#[test]
fn a_late_alert_is_at_the_current_state() {
    let mut c = core();
    fired(&mut c, "Pills", Priority::Medium);
    // The device was off until 30 minutes after it went overdue.
    let p = Alerter::new()
        .pass(&mut c, T0 + HOUR + 30 * MIN, false)
        .unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Insistent);
    assert_eq!(shown(&p.commands)[0].body, "Overdue");
}

#[test]
fn alert_events_need_a_reader_of_format_4() {
    let e = hab_core::Event::OccurrenceAlerted {
        occurrence_id: "x".into(),
        style: AlertStyle::Gentle,
    };
    assert_eq!(e.format(), 4);
    assert_eq!(hab_core::FORMAT_VERSION, 4);
}
