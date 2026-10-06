//! What a device alerts about and when, through the core's own API.

use hab_core::{
    AlertStyle, Alerter, Command, Core, EditReminder, Notification, Priority, PrioritySettings,
    Urgency, ACTION_ACKNOWLEDGE, ACTION_DONE, ACTION_SKIP, ACTION_SNOOZE,
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
fn notifications_carry_done_snooze_and_skip() {
    let mut c = core();
    fired(&mut c, "Water", Priority::Low);
    let pass = Alerter::new().pass(&mut c, T0, false).unwrap();
    let n = shown(&pass.commands)[0].clone();
    let keys: Vec<_> = n.actions.iter().map(|a| a.0).collect();
    assert_eq!(keys, [ACTION_DONE, ACTION_SNOOZE, ACTION_SKIP]);
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
        // Alarm joins after an hour overdue; until then it still repeats.
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
    let p = a.pass(&mut c, T0 + 10 * MIN, false).unwrap();
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
    assert_eq!(hab_core::FORMAT_VERSION, 8);
}

// --- The alarm (#42) ---

fn stops(commands: &[Command]) -> usize {
    commands
        .iter()
        .filter(|c| matches!(c, Command::StopRinging { .. }))
        .count()
}

#[test]
fn an_alarm_is_critical_never_times_out_and_leaves_its_sound_to_the_app() {
    let mut c = core();
    fired(&mut c, "Meds", Priority::High);
    let pass = Alerter::new().pass(&mut c, T0, false).unwrap();
    let n = shown(&pass.commands)[0].clone();
    assert_eq!(n.style, AlertStyle::Alarm);
    assert_eq!(n.urgency, Urgency::Critical);
    assert_eq!(n.timeout_ms, 0, "never expires");
    assert_eq!(n.sound, None, "the app plays its own looping sound");
    let keys: Vec<_> = n.actions.iter().map(|a| a.0).collect();
    assert_eq!(keys, [ACTION_DONE, ACTION_SNOOZE, ACTION_ACKNOWLEDGE]);
}

#[test]
fn medium_becomes_an_alarm_an_hour_after_it_went_overdue() {
    let mut c = core();
    fired(&mut c, "Pills", Priority::Medium);
    let mut a = Alerter::new();
    let overdue = T0 + HOUR;
    let p = a.pass(&mut c, overdue, false).unwrap();
    assert_eq!(shown(&p.commands)[0].urgency, Urgency::Normal);
    let p = a.pass(&mut c, overdue + HOUR, false).unwrap();
    let n = shown(&p.commands)[0].clone();
    assert_eq!(
        (n.style, n.urgency, n.timeout_ms),
        (AlertStyle::Alarm, Urgency::Critical, 0)
    );
}

#[test]
fn an_alarm_with_no_ring_duration_rings_until_someone_acts_and_repeats_each_interval() {
    let mut c = core();
    fired(&mut c, "Meds", Priority::High);
    let mut a = Alerter::new();
    let p = a.pass(&mut c, T0, false).unwrap();
    assert_eq!(p.next_at, Some(T0 + 10 * MIN));
    assert_eq!(stops(&p.commands), 0);
    assert!(a
        .pass(&mut c, T0 + 10 * MIN - 1, false)
        .unwrap()
        .commands
        .is_empty());
    for k in 1..=3 {
        let p = a.pass(&mut c, T0 + k * 10 * MIN, false).unwrap();
        assert_eq!(shown(&p.commands).len(), 1, "repeat {k}");
        assert_eq!(stops(&p.commands), 0, "it never stops by itself");
    }
}

fn rings_for_two_minutes(p: Priority) -> PrioritySettings {
    PrioritySettings {
        ring_duration: Some(2 * MIN),
        ..p.settings()
    }
}

#[test]
fn an_alarm_stops_ringing_after_its_ring_duration_and_rings_again_at_the_repeat() {
    let mut c = core();
    fired(&mut c, "Meds", Priority::High);
    let mut a = Alerter::with_settings(rings_for_two_minutes);
    let p = a.pass(&mut c, T0, false).unwrap();
    assert_eq!(p.next_at, Some(T0 + 2 * MIN), "wakes to stop the sound");
    assert!(a
        .pass(&mut c, T0 + 2 * MIN - 1, false)
        .unwrap()
        .commands
        .is_empty());
    let p = a.pass(&mut c, T0 + 2 * MIN, false).unwrap();
    assert_eq!(
        p.commands,
        [Command::StopRinging {
            occurrence_id: shown_id(&c)
        }]
    );
    assert_eq!(p.next_at, Some(T0 + 10 * MIN), "next is the repeat");
    // Only once, and nothing else until the repeat rings it again.
    assert!(a
        .pass(&mut c, T0 + 5 * MIN, false)
        .unwrap()
        .commands
        .is_empty());
    let p = a.pass(&mut c, T0 + 10 * MIN, false).unwrap();
    assert_eq!(shown(&p.commands).len(), 1);
    let p = a.pass(&mut c, T0 + 12 * MIN, false).unwrap();
    assert_eq!(stops(&p.commands), 1);
}

fn shown_id(c: &Core) -> String {
    c.inbox(T0).overdue[0].occurrence_id.clone()
}

#[test]
fn maximum_stays_an_alarm_through_do_not_disturb_and_high_goes_silent() {
    let mut c = core();
    fired(&mut c, "Meds", Priority::Maximum);
    fired(&mut c, "Bins", Priority::High);
    let p = Alerter::new().pass(&mut c, T0, true).unwrap();
    for n in shown(&p.commands) {
        match n.title.as_str() {
            "Meds" => assert_eq!((n.urgency, n.timeout_ms), (Urgency::Critical, 0)),
            _ => assert_eq!((n.style, n.urgency), (AlertStyle::Silent, Urgency::Low)),
        }
    }
}

#[test]
fn an_alarm_downgraded_by_do_not_disturb_is_a_silent_notification() {
    // High alarmed, then Do Not Disturb came on: the repeat is silent, which
    // the platform treats as the alarm ending (its sound and window stop).
    let mut c = core();
    fired(&mut c, "Bins", Priority::High);
    let mut a = Alerter::new();
    assert_eq!(
        shown(&a.pass(&mut c, T0, false).unwrap().commands)[0].style,
        AlertStyle::Alarm
    );
    let p = a.pass(&mut c, T0 + MIN, true).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Silent);
}

#[test]
fn acknowledging_while_due_quiets_until_it_goes_overdue() {
    let mut c = core();
    let occ = fired(&mut c, "Pills", Priority::Medium);
    let mut a = Alerter::new();
    a.pass(&mut c, T0, false).unwrap();
    c.acknowledge(&occ, T0 + MIN).unwrap();
    let p = a.pass(&mut c, T0 + MIN, false).unwrap();
    assert_eq!(
        p.commands,
        [Command::Close {
            occurrence_id: occ.clone()
        }]
    );
    assert_eq!(p.next_at, Some(T0 + HOUR), "wakes when it goes overdue");
    assert!(a
        .pass(&mut c, T0 + HOUR - 1, false)
        .unwrap()
        .commands
        .is_empty());
    // Overdue: escalation resumes at once.
    let p = a.pass(&mut c, T0 + HOUR, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Insistent);
    assert!(c.state().occurrences[&occ].is_open(), "never closed");
}

#[test]
fn acknowledging_while_overdue_quiets_for_one_overdue_interval() {
    let mut c = core();
    let occ = fired(&mut c, "Meds", Priority::High);
    let mut a = Alerter::new();
    a.pass(&mut c, T0, false).unwrap();
    let acked = T0 + 3 * MIN;
    c.acknowledge(&occ, acked).unwrap();
    let p = a.pass(&mut c, acked, false).unwrap();
    assert_eq!(
        p.commands,
        [Command::Close {
            occurrence_id: occ.clone()
        }]
    );
    assert_eq!(p.next_at, Some(acked + 10 * MIN));
    assert!(a
        .pass(&mut c, acked + 10 * MIN - 1, false)
        .unwrap()
        .commands
        .is_empty());
    // The interval is up: it rings again, and then repeats as usual.
    let p = a.pass(&mut c, acked + 10 * MIN, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);
    assert_eq!(p.next_at, Some(acked + 20 * MIN));
}

#[test]
fn acknowledging_again_restarts_the_quiet_period_and_uses_the_latest() {
    let mut c = core();
    let occ = fired(&mut c, "Meds", Priority::High);
    let mut a = Alerter::new();
    c.acknowledge(&occ, T0 + MIN).unwrap();
    c.acknowledge(&occ, T0 + 4 * MIN).unwrap();
    let ack = c.state().occurrences[&occ].acknowledged_at;
    assert_eq!(ack, Some(T0 + 4 * MIN));
    assert!(a
        .pass(&mut c, T0 + 11 * MIN, false)
        .unwrap()
        .commands
        .is_empty());
    assert_eq!(
        shown(&a.pass(&mut c, T0 + 14 * MIN, false).unwrap().commands).len(),
        1
    );
}

#[test]
fn acknowledgements_are_recorded_with_who_and_when_and_need_no_new_format() {
    let mut c = core();
    c.join("u1", "dev-9").unwrap();
    let occ = fired(&mut c, "Meds", Priority::High);
    c.acknowledge(&occ, T0 + 5).unwrap();
    let sent = c.unsent().unwrap();
    let ack = sent
        .iter()
        .find(|o| {
            let p: hab_core::Payload = serde_json::from_slice(&o.payload).unwrap();
            p.event["type"] == "occurrence_acknowledged"
        })
        .expect("the acknowledgement is an event in the list's history");
    let payload: hab_core::Payload = serde_json::from_slice(&ack.payload).unwrap();
    assert_eq!(
        (payload.author.as_str(), payload.recorded_at),
        ("u1", T0 + 5)
    );
    assert_eq!(ack.format, 1, "no new format: the time is the event's own");
    assert_eq!(hab_core::FORMAT_VERSION, 8);
}

#[test]
fn one_tap_snooze_uses_the_priority_interval() {
    let mut c = core();
    let medium = fired(&mut c, "Pills", Priority::Medium);
    let high = fired(&mut c, "Meds", Priority::High);
    let low = fired(&mut c, "Water", Priority::Low);
    assert_eq!(
        c.snooze_for_interval(&medium, T0 + MIN).unwrap(),
        T0 + MIN + HOUR
    );
    assert_eq!(
        c.snooze_for_interval(&high, T0 + MIN).unwrap(),
        T0 + MIN + 10 * MIN
    );
    assert_eq!(
        c.snooze_for_interval(&low, T0 + MIN).unwrap(),
        T0 + MIN + 24 * HOUR
    );
    // Once Medium is overdue the interval is the overdue one.
    let mut c = core();
    let medium = fired(&mut c, "Pills", Priority::Medium);
    let now = T0 + HOUR + MIN;
    assert_eq!(c.snooze_for_interval(&medium, now).unwrap(), now + 10 * MIN);
}

#[test]
fn the_alarm_window_view_names_the_list_priority_and_due_time() {
    let mut c = core();
    let occ = fired(&mut c, "Meds", Priority::High);
    let v = c.occurrence_view(&occ).unwrap();
    assert_eq!(
        (v.title.as_str(), v.priority, v.scheduled_at),
        ("Meds", Priority::High, T0)
    );
    assert_eq!(v.list_name, None, "the personal list");
    c.complete(&occ, T0 + 1).unwrap();
    assert!(c.occurrence_view(&occ).is_none());
}
