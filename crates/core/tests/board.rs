//! The Board's data (#53): every live reminder with what the window needs to
//! put it in a column, and duplicating a reminder.

use hab_core::{Core, Countdown, CountdownUnit, Faking, Schedule};

const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
// 2026-10-03T00:00:00Z
const D0: i64 = 1_790_985_600;

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

fn daily(c: &mut Core, title: &str, now: i64) -> String {
    c.create_recurring_reminder(
        title,
        vec![Schedule {
            start: "2026-10-01T07:00:00".into(),
            rule: "FREQ=DAILY".into(),
        }],
        Some("UTC"),
        now,
    )
    .unwrap()
}

#[test]
fn every_live_reminder_is_a_card_by_title_and_deleted_ones_are_not() {
    let mut c = core();
    let now = D0 + 12 * HOUR;
    c.create_reminder("Zebra", now + HOUR, now).unwrap();
    c.create_reminder("Apple", now + 2 * HOUR, now).unwrap();
    let gone = c.create_reminder("Gone", now + 3 * HOUR, now).unwrap();
    c.delete_reminder(&gone, now).unwrap();
    let titles: Vec<String> = c.board(now).into_iter().map(|b| b.title).collect();
    assert_eq!(titles, vec!["Apple", "Zebra"]);
}

#[test]
fn a_scheduled_reminder_has_its_next_expected_occurrence_and_no_open_one() {
    let mut c = core();
    let now = D0 + 12 * HOUR;
    let rid = daily(&mut c, "Medicine", D0);
    let card = &c.board(now)[0];
    assert_eq!(card.reminder_id, rid);
    assert!(card.repeats && card.open.is_none() && card.pause.is_none());
    let next = card.next.as_ref().unwrap();
    assert_eq!(next.scheduled_at, D0 + DAY + 7 * HOUR);
    assert!(next.can_close_early);
    assert_eq!(card.faking, Faking::None);
    assert_eq!(card.last_closed_at, None);
}

#[test]
fn a_reminder_with_an_open_occurrence_has_it_and_no_next() {
    let mut c = core();
    let t = D0 + 9 * HOUR;
    let rid = c.create_reminder("Call", t, D0).unwrap();
    c.tick(t).unwrap();
    let card = &c.board(t + 60)[0];
    assert_eq!(
        card.open.as_ref().unwrap().occurrence_id,
        format!("{rid}@{t}")
    );
    assert!(card.next.is_none());
    assert!(!card.repeats);
}

#[test]
fn a_closed_one_off_has_nothing_next_and_when_it_was_closed() {
    let mut c = core();
    let t = D0 + 9 * HOUR;
    let rid = c.create_reminder("Call", t, D0).unwrap();
    c.tick(t).unwrap();
    c.complete(&format!("{rid}@{t}"), t + 600).unwrap();
    let card = &c.board(t + 700)[0];
    assert!(card.open.is_none() && card.next.is_none());
    assert_eq!(card.last_closed_at, Some(t + 600));
}

#[test]
fn a_one_off_closed_early_and_undone_is_expected_again() {
    let mut c = core();
    let t = D0 + 9 * HOUR;
    let rid = c.create_reminder("Call", t, D0).unwrap();
    c.complete_expected(&rid, D0 + HOUR, D0 + HOUR).unwrap();
    assert_eq!(c.board(D0 + 2 * HOUR)[0].last_closed_at, Some(D0 + HOUR));
    c.undo(&format!("{rid}@{t}"), D0 + 2 * HOUR).unwrap();
    let card = &c.board(D0 + 3 * HOUR)[0];
    assert_eq!(card.next.as_ref().unwrap().scheduled_at, t);
    assert_eq!(card.last_closed_at, None);
}

#[test]
fn a_paused_reminder_says_what_pauses_it_and_the_pause_ends_with_its_time() {
    let mut c = core();
    let now = D0 + 12 * HOUR;
    let rid = daily(&mut c, "Medicine", D0);
    c.pause_reminder(&rid, Some(D0 + 3 * DAY), now).unwrap();
    let cause = c.board(now + 60)[0].pause.unwrap();
    assert_eq!((cause.until, cause.list), (Some(D0 + 3 * DAY), false));
    assert!(c.board(D0 + 3 * DAY + 1)[0].pause.is_none());

    let personal = c.personal_list_id().to_string();
    c.resume_reminder(&rid, now + 120).unwrap();
    c.pause_list(&personal, None, now + 180).unwrap();
    let cause = c.board(now + 240)[0].pause.unwrap();
    assert!(cause.list && cause.until.is_none());
}

#[test]
fn pausing_skips_the_open_occurrence_so_the_card_has_none() {
    let mut c = core();
    let t = D0 + 9 * HOUR;
    c.create_reminder("Call", t, D0).unwrap();
    c.tick(t).unwrap();
    let rid = c.board(t)[0].reminder_id.clone();
    // Pausing skips the open occurrence, so a card is only in Overdue or Due
    // under a pause when something reopened it (an undo, or another device).
    c.pause_reminder(&rid, None, t + 60).unwrap();
    let card = &c.board(t + 120)[0];
    assert!(card.pause.is_some());
    assert!(card.open.is_none(), "the pause skipped it");
}

#[test]
fn a_countdown_card_counts_from_its_last_closing() {
    let mut c = core();
    let rid = c
        .create_countdown_reminder(
            "Plants",
            Countdown {
                amount: 3,
                unit: CountdownUnit::Days,
                at: None,
            },
            Some("UTC"),
            Some(D0),
            D0,
        )
        .unwrap();
    let card = &c.board(D0 + HOUR)[0];
    assert_eq!(card.reminder_id, rid);
    assert!(card.repeats);
    assert_eq!(card.next.as_ref().unwrap().scheduled_at, D0 + 3 * DAY);
}

// ---- Duplicate ----

#[test]
fn duplicating_a_repeating_reminder_copies_what_makes_it_up_but_not_its_pause() {
    let mut c = core();
    let now = D0 + 12 * HOUR;
    let rid = daily(&mut c, "Medicine", D0);
    c.pause_reminder(&rid, None, now).unwrap();
    let copy = c.duplicate_reminder(&rid, now + 60).unwrap();
    assert_ne!(copy, rid);
    let (a, b) = (
        c.reminder_view(&rid).unwrap(),
        c.reminder_view(&copy).unwrap(),
    );
    assert_eq!(b.title, "Medicine (copy)");
    assert_eq!(b.trigger, a.trigger);
    assert_eq!(b.priority, a.priority);
    assert_eq!(b.zone, a.zone);
    assert!(b.pause.is_none());
    assert_eq!(c.board(now + 120).len(), 2);
}

#[test]
fn duplicating_keeps_the_note_priority_and_list_and_dates_a_past_one_off_ahead() {
    let mut c = core();
    let now = D0 + 12 * HOUR;
    let list = c.create_list("Home", None, now).unwrap();
    let rid = c.create_reminder_in(&list, "Call", D0 + HOUR, D0).unwrap();
    c.edit_reminder(
        &rid,
        hab_core::EditReminder {
            note: Some("the plumber".into()),
            priority: Some(hab_core::Priority::High),
            ..Default::default()
        },
        D0,
    )
    .unwrap();
    let copy = c.duplicate_reminder(&rid, now).unwrap();
    let v = c.reminder_view(&copy).unwrap();
    assert_eq!(v.list_id, list);
    assert_eq!(v.note, "the plumber");
    assert_eq!(v.priority, hab_core::Priority::High);
    assert_eq!(
        v.trigger,
        hab_core::TriggerView::OneOff {
            fire_at: now + HOUR
        }
    );
}

#[test]
fn duplicating_a_countdown_starts_it_counting_from_now() {
    let mut c = core();
    let rid = c
        .create_countdown_reminder(
            "Plants",
            Countdown {
                amount: 3,
                unit: CountdownUnit::Days,
                at: None,
            },
            Some("UTC"),
            Some(D0 - 10 * DAY),
            D0,
        )
        .unwrap();
    let copy = c.duplicate_reminder(&rid, D0 + HOUR).unwrap();
    let card = c
        .board(D0 + 2 * HOUR)
        .into_iter()
        .find(|b| b.reminder_id == copy)
        .unwrap();
    assert_eq!(card.next.unwrap().scheduled_at, D0 + HOUR + 3 * DAY);
}

#[test]
fn duplicating_something_that_does_not_exist_is_an_error() {
    let mut c = core();
    assert!(c.duplicate_reminder("nope", D0).is_err());
}
