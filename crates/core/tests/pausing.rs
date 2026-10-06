//! Pausing reminders and lists (#46): what a pause skips, how the history
//! attributes it, resuming early, and several devices pausing and resuming
//! out of touch. Time is passed in, so every test says what the clock reads.

use std::collections::BTreeMap;

use hab_core::{
    Alerter, Change, ClosingKind, Command, Core, Countdown, CountdownUnit, Error, Event,
    HistoryWhat, Hlc, Outcome, Pause, PauseCause, Priority, Schedule, Setting, UndoOutcome,
    FORMAT_VERSION,
};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

const USER: &str = "u1";

fn at(local: &str) -> i64 {
    let dt: DateTime = local.parse().unwrap();
    TimeZone::UTC.to_zoned(dt).unwrap().timestamp().as_second()
}

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

fn daily(c: &mut Core, now: i64) -> String {
    c.create_recurring_reminder(
        "Medicine",
        vec![Schedule {
            start: "2026-10-01T07:00:00".into(),
            rule: "FREQ=DAILY".into(),
        }],
        Some("UTC"),
        now,
    )
    .unwrap()
}

fn plants(c: &mut Core, last: i64) -> String {
    c.create_countdown_reminder(
        "Plants",
        Countdown {
            amount: 3,
            unit: CountdownUnit::Days,
            at: None,
        },
        Some("UTC"),
        Some(last),
        last,
    )
    .unwrap()
}

fn occ(rid: &str, local: &str) -> String {
    format!("{rid}@{}", at(local))
}

fn closing(c: &Core, id: &str) -> Option<(ClosingKind, i64)> {
    c.state().occurrences[id]
        .closing
        .as_ref()
        .map(|c| (c.kind, c.at))
}

fn by_reminder(until: Option<i64>) -> PauseCause {
    PauseCause { until, list: false }
}

// ---- What a pause skips ----

#[test]
fn instances_in_a_pause_are_skipped_not_fired_and_the_history_says_it_was_the_pause() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let day1 = c.tick(at("2026-10-01T07:00:00")).unwrap();
    c.complete(&day1[0].occurrence_id, at("2026-10-01T07:05:00"))
        .unwrap();

    // Paused that evening until the morning of the 4th.
    let until = at("2026-10-04T00:00:00");
    c.pause_reminder(&rid, Some(until), at("2026-10-01T20:00:00"))
        .unwrap();

    for day in ["2026-10-02", "2026-10-03"] {
        let when = at(&format!("{day}T07:00:00"));
        assert!(c.tick(when).unwrap().is_empty(), "nothing fires on {day}");
        let id = format!("{rid}@{when}");
        // Skipped as of its own time, by the pause.
        assert_eq!(closing(&c, &id), Some((ClosingKind::Skipped, when)));
        let v = c.closed_occurrence(&id).unwrap();
        assert_eq!(v.kind, ClosingKind::Skipped);
        assert_eq!(v.outcome, Outcome::Skipped);
        assert_eq!(v.paused, Some(by_reminder(Some(until))));
        assert_eq!(v.history.len(), 1);
        assert_eq!(v.history[0].what, HistoryWhat::Skipped);
        assert_eq!(v.history[0].paused, Some(by_reminder(Some(until))));
        assert!(v.can_undo, "a skip can be undone");
    }
    assert!(c.inbox(at("2026-10-03T08:00:00")).due.is_empty());

    // Once the pause is over it fires as usual.
    let day4 = c.tick(at("2026-10-04T07:00:00")).unwrap();
    assert_eq!(day4.len(), 1);
    assert_eq!(
        day4[0].occurrence_id,
        occ(&rid, "2026-10-04T07:00:00"),
        "fires on its own instance"
    );
    // An ordinary skip is not attributed to anything.
    c.skip(&day4[0].occurrence_id, None, at("2026-10-04T07:10:00"))
        .unwrap();
    assert_eq!(
        c.closed_occurrence(&day4[0].occurrence_id).unwrap().paused,
        None
    );
}

#[test]
fn skips_made_while_paused_show_in_earlier_today_with_the_pause() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let until = at("2026-10-05T00:00:00");
    c.pause_reminder(&rid, Some(until), at("2026-10-01T06:30:00"))
        .unwrap();
    let now = at("2026-10-02T09:00:00");
    c.tick(now).unwrap();
    // The 1st's 07:00 and the 2nd's were both in the pause.
    let earlier = c.inbox(now).earlier_today;
    assert_eq!(earlier.len(), 1);
    assert_eq!(earlier[0].kind, ClosingKind::Skipped);
    assert_eq!(earlier[0].paused, Some(by_reminder(Some(until))));
}

#[test]
fn a_device_that_was_asleep_through_a_pause_skips_what_it_covered_and_fires_the_latest() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    c.pause_reminder(
        &rid,
        Some(at("2026-10-04T00:00:00")),
        at("2026-10-01T06:30:00"),
    )
    .unwrap();
    // Nothing ticks until noon on the 5th.
    let fired = c.tick(at("2026-10-05T12:00:00")).unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].occurrence_id, occ(&rid, "2026-10-05T07:00:00"));
    for day in ["2026-10-01", "2026-10-02", "2026-10-03"] {
        let id = occ(&rid, &format!("{day}T07:00:00"));
        assert_eq!(c.closed_occurrence(&id).unwrap().kind, ClosingKind::Skipped);
        assert!(c.closed_occurrence(&id).unwrap().paused.is_some());
    }
    // The 4th came after the pause: it is missed, as the 5th fired.
    let id = occ(&rid, "2026-10-04T07:00:00");
    assert_eq!(c.closed_occurrence(&id).unwrap().kind, ClosingKind::Missed);
}

#[test]
fn a_one_off_that_comes_due_in_a_pause_is_skipped_and_finished() {
    let mut c = core();
    let fire = at("2026-10-02T12:00:00");
    let rid = c
        .create_reminder("Call the plumber", fire, at("2026-10-01T09:00:00"))
        .unwrap();
    c.pause_reminder(
        &rid,
        Some(at("2026-10-03T00:00:00")),
        at("2026-10-01T10:00:00"),
    )
    .unwrap();
    assert!(c.tick(fire).unwrap().is_empty());
    let id = format!("{rid}@{fire}");
    assert_eq!(closing(&c, &id), Some((ClosingKind::Skipped, fire)));
    assert!(c.state().is_finished(&rid));
    assert!(!c.closed_occurrence(&id).unwrap().paused.unwrap().list);
}

#[test]
fn a_countdown_that_runs_out_in_a_pause_is_skipped_and_restarts_from_then() {
    let mut c = core();
    let rid = plants(&mut c, at("2026-10-01T08:00:00"));
    // Runs out on the 4th at 08:00, the 7th, the 10th.
    c.pause_reminder(
        &rid,
        Some(at("2026-10-08T00:00:00")),
        at("2026-10-02T09:00:00"),
    )
    .unwrap();
    let fired = c.tick(at("2026-10-10T09:00:00")).unwrap();
    for day in ["2026-10-04", "2026-10-07"] {
        let id = occ(&rid, &format!("{day}T08:00:00"));
        let v = c.closed_occurrence(&id).unwrap();
        assert_eq!(v.kind, ClosingKind::Skipped);
        assert!(v.paused.is_some());
        // Each skip is as of when it ran out, so the next counts from there.
        assert_eq!(v.at, at(&format!("{day}T08:00:00")));
    }
    // The 10th's is after the pause: it fires, a whole period after the
    // last skip rather than at the moment the pause ended.
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].occurrence_id, occ(&rid, "2026-10-10T08:00:00"));
}

#[test]
fn nothing_paused_is_expected_and_the_device_still_wakes_to_skip_it() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let day1 = c.tick(at("2026-10-01T07:00:00")).unwrap();
    c.complete(&day1[0].occurrence_id, at("2026-10-01T07:05:00"))
        .unwrap();
    let now = at("2026-10-01T20:00:00");
    c.pause_reminder(&rid, Some(at("2026-10-03T00:00:00")), now)
        .unwrap();
    let times: Vec<i64> = c
        .expected(now, at("2026-10-05T23:00:00"))
        .into_iter()
        .map(|e| e.scheduled_at)
        .collect();
    assert_eq!(
        times,
        vec![
            at("2026-10-03T07:00:00"),
            at("2026-10-04T07:00:00"),
            at("2026-10-05T07:00:00")
        ],
        "the 2nd's 07:00 falls in the pause"
    );
    // The skip still has to be recorded, so the scheduler wakes for it.
    assert_eq!(c.next_fire_at(), Some(at("2026-10-02T07:00:00")));
    c.tick(at("2026-10-02T07:00:00")).unwrap();
    assert_eq!(c.next_fire_at(), Some(at("2026-10-03T07:00:00")));
}

// ---- Resuming ----

#[test]
fn resuming_early_returns_to_normal_firing_from_the_next_instance() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    c.pause_reminder(
        &rid,
        Some(at("2026-10-10T00:00:00")),
        at("2026-10-01T20:00:00"),
    )
    .unwrap();
    assert!(c.tick(at("2026-10-02T07:00:00")).unwrap().is_empty());
    assert!(c.is_paused(&rid, at("2026-10-02T12:00:00")));

    c.resume_reminder(&rid, at("2026-10-03T05:00:00")).unwrap();
    assert!(!c.is_paused(&rid, at("2026-10-03T05:00:01")));
    // The 2nd stays skipped, by the pause; the 3rd fires.
    let id2 = occ(&rid, "2026-10-02T07:00:00");
    assert_eq!(
        c.closed_occurrence(&id2).unwrap().kind,
        ClosingKind::Skipped
    );
    let fired = c.tick(at("2026-10-03T07:00:00")).unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].occurrence_id, occ(&rid, "2026-10-03T07:00:00"));
    // And the expected ones are back.
    assert!(!c
        .expected(at("2026-10-03T08:00:00"), at("2026-10-05T23:00:00"))
        .is_empty());
}

#[test]
fn resuming_before_the_device_ticked_still_skips_what_the_pause_covered() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let day1 = c.tick(at("2026-10-01T07:00:00")).unwrap();
    c.complete(&day1[0].occurrence_id, at("2026-10-01T07:05:00"))
        .unwrap();
    c.pause_reminder(
        &rid,
        Some(at("2026-10-10T00:00:00")),
        at("2026-10-01T20:00:00"),
    )
    .unwrap();
    // The device slept through the 2nd's 07:00 and was resumed at 09:00,
    // before it ticked: the 2nd was in the pause and doesn't fire late.
    c.resume_reminder(&rid, at("2026-10-02T09:00:00")).unwrap();
    assert!(c.tick(at("2026-10-02T09:00:01")).unwrap().is_empty());
    let id = occ(&rid, "2026-10-02T07:00:00");
    assert!(c.closed_occurrence(&id).unwrap().paused.is_some());
    assert_eq!(c.tick(at("2026-10-03T07:00:00")).unwrap().len(), 1);
}

#[test]
fn resuming_what_is_not_paused_does_nothing() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let before = c.unsent().unwrap().len();
    c.resume_reminder(&rid, at("2026-10-01T07:00:00")).unwrap();
    let personal = c.personal_list_id().to_string();
    c.resume_list(&personal, at("2026-10-01T07:00:00")).unwrap();
    assert_eq!(c.unsent().unwrap().len(), before);
}

#[test]
fn a_pause_can_be_moved_without_losing_where_it_began_and_must_end_in_the_future() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let t = at("2026-10-01T20:00:00");
    c.pause_reminder(&rid, Some(at("2026-10-03T00:00:00")), t)
        .unwrap();
    // Extended the next morning: it still began the evening before.
    c.pause_reminder(&rid, None, at("2026-10-02T09:00:00"))
        .unwrap();
    let r = &c.state().reminders[&rid];
    assert_eq!(
        r.pause,
        Some(Pause {
            from: t,
            until: None
        })
    );
    assert!(matches!(
        c.pause_reminder(&rid, Some(t), t),
        Err(Error::PauseInThePast)
    ));
    let personal = c.personal_list_id().to_string();
    assert!(matches!(
        c.pause_list(&personal, Some(t - 1), t),
        Err(Error::PauseInThePast)
    ));
    assert!(matches!(
        c.pause_reminder("nope", None, t),
        Err(Error::NoReminder(_))
    ));
}

// ---- Pausing what is open ----

#[test]
fn pausing_skips_the_open_occurrence_and_the_history_says_so() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let day1 = at("2026-10-01T07:00:00");
    let fired = c.tick(day1).unwrap();
    let id = fired[0].occurrence_id.clone();
    let mut alerter = Alerter::new();
    assert!(matches!(
        alerter.pass(&mut c, day1, false).unwrap().commands[0],
        Command::Show(_)
    ));

    // "Pause" in its More menu, until the 5th.
    let now = at("2026-10-01T07:30:00");
    let until = at("2026-10-05T00:00:00");
    c.pause_reminder(&rid, Some(until), now).unwrap();
    assert_eq!(closing(&c, &id), Some((ClosingKind::Skipped, now)));
    let v = c.closed_occurrence(&id).unwrap();
    assert_eq!(v.paused, Some(by_reminder(Some(until))));
    // Its alert goes.
    let pass = alerter.pass(&mut c, now, false).unwrap();
    assert_eq!(
        pass.commands,
        vec![Command::Close {
            occurrence_id: id.clone()
        }]
    );
    assert!(c.inbox(now).due.is_empty());

    // Undoing the skip reopens it, but it is quiet while the pause lasts.
    let outcome = c.undo(&id, now + 60).unwrap();
    assert_eq!(outcome, UndoOutcome::Reopened);
    let inbox = c.inbox(now + 60);
    assert!(inbox.due.is_empty() && inbox.overdue.is_empty());
    assert_eq!(inbox.paused.len(), 1);
    assert_eq!(inbox.paused[0].item.occurrence_id, id);
    assert_eq!(inbox.paused[0].pause, by_reminder(Some(until)));
    let pass = alerter.pass(&mut c, now + 60, false).unwrap();
    assert!(pass.commands.is_empty());
    assert_eq!(
        pass.next_at,
        Some(until),
        "alerts again when the pause ends"
    );
    // It can still be acted on.
    c.complete(&id, now + 120).unwrap();
}

#[test]
fn an_alert_standing_for_a_reminder_paused_elsewhere_is_closed_and_comes_back_after() {
    let mut w = World::new(2);
    let rid = daily(w.dev(1), at("2026-10-01T06:00:00"));
    w.sync_all();
    // Device 1 pauses at 06:30 until the 3rd. Device 2 doesn't hear.
    let until = at("2026-10-03T00:00:00");
    w.dev(1)
        .pause_reminder(&rid, Some(until), at("2026-10-01T06:30:00"))
        .unwrap();
    w.upload(1);
    // Device 2 fires and alerts at 07:00, not knowing.
    let day1 = at("2026-10-01T07:00:00");
    let id = occ(&rid, "2026-10-01T07:00:00");
    let mut alerter = Alerter::new();
    assert_eq!(w.dev(2).tick(day1).unwrap().len(), 1);
    assert!(matches!(
        alerter.pass(w.dev(2), day1, false).unwrap().commands[0],
        Command::Show(_)
    ));
    // It hears. The alert is taken down, and the occurrence waits, quiet.
    w.download(2);
    let pass = alerter.pass(w.dev(2), day1 + 10, false).unwrap();
    assert_eq!(
        pass.commands,
        vec![Command::Close {
            occurrence_id: id.clone()
        }]
    );
    assert_eq!(pass.next_at, Some(until));
    let inbox = w.dev(2).inbox(day1 + 10);
    assert_eq!(inbox.paused.len(), 1);
    // When the pause ends, an occurrence still open alerts again.
    let pass = alerter.pass(w.dev(2), until, false).unwrap();
    assert!(matches!(&pass.commands[..], [Command::Show(n)] if n.occurrence_id == id));
}

// ---- Lists ----

#[test]
fn a_paused_list_sets_aside_every_reminder_in_it_including_ones_moved_in() {
    let mut c = core();
    let personal = c.personal_list_id().to_string();
    let home = c
        .create_list("Home", None, at("2026-10-01T05:00:00"))
        .unwrap();
    let a = c
        .create_recurring_reminder_in(
            &home,
            "Bins",
            vec![Schedule {
                start: "2026-10-01T07:00:00".into(),
                rule: "FREQ=DAILY".into(),
            }],
            Some("UTC"),
            at("2026-10-01T06:00:00"),
        )
        .unwrap();
    let b = daily(&mut c, at("2026-10-01T06:00:00"));
    let day1 = at("2026-10-01T07:00:00");
    let fired = c.tick(day1).unwrap();
    assert_eq!(fired.len(), 2);

    // Paused from the list itself, until the 4th.
    let now = at("2026-10-01T08:00:00");
    let until = at("2026-10-04T00:00:00");
    c.pause_list(&home, Some(until), now).unwrap();
    assert_eq!(
        c.lists().iter().find(|l| l.id == home).unwrap().pause,
        Some(Pause {
            from: now,
            until: Some(until)
        })
    );
    // The open occurrence in the list was skipped, by the list.
    let open = occ(&a, "2026-10-01T07:00:00");
    let v = c.closed_occurrence(&open).unwrap();
    assert_eq!(
        v.paused,
        Some(PauseCause {
            until: Some(until),
            list: true
        })
    );
    // The other list's reminder is untouched.
    assert!(c.state_of(&personal).unwrap().occurrences[&occ(&b, "2026-10-01T07:00:00")].is_open());

    // The next day's: skipped in the paused list, fired in the other.
    let day2 = at("2026-10-02T07:00:00");
    let fired = c.tick(day2).unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].reminder_id, b);
    let skipped = c
        .closed_occurrence(&occ(&a, "2026-10-02T07:00:00"))
        .unwrap();
    assert!(skipped.paused.unwrap().list);

    // Moved out, a reminder is no longer paused; moved in, it is.
    c.move_reminder(&b, &home, at("2026-10-02T08:00:00"))
        .unwrap();
    let day3 = at("2026-10-03T07:00:00");
    assert!(c.tick(day3).unwrap().is_empty());
    assert!(c.is_paused(&b, day3));
    let paused: Vec<_> = c
        .paused_reminders(day3)
        .into_iter()
        .map(|p| p.reminder_id)
        .collect();
    assert!(paused.contains(&a) && paused.contains(&b));

    // Resumed, both fire again from the next instance.
    c.resume_list(&home, at("2026-10-03T12:00:00")).unwrap();
    assert!(c.paused_reminders(at("2026-10-03T12:00:01")).is_empty());
    assert_eq!(c.tick(at("2026-10-04T07:00:00")).unwrap().len(), 2);
}

#[test]
fn a_list_and_a_reminder_can_both_be_paused_and_resuming_one_leaves_the_other() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let personal = c.personal_list_id().to_string();
    let now = at("2026-10-01T20:00:00");
    c.pause_reminder(&rid, Some(at("2026-10-03T00:00:00")), now)
        .unwrap();
    c.pause_list(&personal, Some(at("2026-10-05T00:00:00")), now)
        .unwrap();
    let v = c.reminder_view(&rid).unwrap();
    assert!(v.pause.is_some() && v.list_pause.is_some());
    c.resume_reminder(&rid, now + 60).unwrap();
    assert!(c.is_paused(&rid, now + 120), "its list still pauses it");
    let p = &c.paused_reminders(now + 120)[0];
    assert!(p.pause.list);
    assert_eq!(p.from, now);
    c.resume_list(&personal, now + 180).unwrap();
    assert!(!c.is_paused(&rid, now + 240));
}

// ---- Several devices ----

#[test]
fn devices_pausing_the_same_reminder_out_of_touch_agree_on_the_later_pause() {
    for order in [[1, 2], [2, 1]] {
        let mut w = World::new(2);
        let rid = daily(w.dev(1), at("2026-10-01T06:00:00"));
        w.sync_all();
        let t = at("2026-10-01T20:00:00");
        w.dev(1)
            .pause_reminder(&rid, Some(at("2026-10-03T00:00:00")), t)
            .unwrap();
        w.dev(2)
            .pause_reminder(&rid, Some(at("2026-10-09T00:00:00")), t + 10)
            .unwrap();
        w.sync_in_order(&order);
        let p1 = w.dev(1).state().reminders[&rid].pause;
        let p2 = w.dev(2).state().reminders[&rid].pause;
        assert_eq!(p1, p2, "converged, uploaded in order {order:?}");
        assert_eq!(p1.unwrap().until, Some(at("2026-10-09T00:00:00")));
        // Both versions are kept: the lost one can be brought back.
        let h = w.dev(1).state().history(&rid, Setting::Pause);
        assert_eq!(h.len(), 2);
        assert!(h[0].current && !h[1].current);
    }
}

#[test]
fn a_pause_and_a_resume_made_out_of_touch_converge_whichever_came_last() {
    for resume_last in [true, false] {
        // However the server numbers them.
        for order in [[1, 2], [2, 1]] {
            let mut w = World::new(2);
            let rid = daily(w.dev(1), at("2026-10-01T06:00:00"));
            w.sync_all();
            let t = at("2026-10-01T20:00:00");
            // Paused, and both devices hear of it.
            w.dev(1)
                .pause_reminder(&rid, Some(at("2026-10-10T00:00:00")), t)
                .unwrap();
            w.sync_all();
            let (resume_at, repause_at) = if resume_last {
                (t + 200, t + 100)
            } else {
                (t + 100, t + 200)
            };
            w.dev(1).resume_reminder(&rid, resume_at).unwrap();
            w.dev(2)
                .pause_reminder(&rid, Some(at("2026-10-12T00:00:00")), repause_at)
                .unwrap();
            w.sync_in_order(&order);
            let a = w.dev(1).state().reminders[&rid].pause;
            let b = w.dev(2).state().reminders[&rid].pause;
            assert_eq!(a, b, "resume last: {resume_last}, order {order:?}");
            if resume_last {
                assert_eq!(a, None, "the later resume wins");
            } else {
                assert_eq!(a.unwrap().until, Some(at("2026-10-12T00:00:00")));
                assert_eq!(a.unwrap().from, t, "the pause kept where it began");
            }
        }
    }
}

#[test]
fn two_devices_skipping_the_same_instance_agree_it_is_one_skip() {
    let mut w = World::new(2);
    let rid = daily(w.dev(1), at("2026-10-01T06:00:00"));
    w.sync_all();
    w.dev(1)
        .pause_reminder(
            &rid,
            Some(at("2026-10-05T00:00:00")),
            at("2026-10-01T20:00:00"),
        )
        .unwrap();
    w.sync_all();
    // Both are awake at 07:00 on the 2nd and record the skip.
    let t = at("2026-10-02T07:00:00");
    assert!(w.dev(1).tick(t).unwrap().is_empty());
    assert!(w.dev(2).tick(t + 3).unwrap().is_empty());
    w.sync_all();
    let id = occ(&rid, "2026-10-02T07:00:00");
    for i in 1..=2 {
        let c = w.dev(i);
        assert_eq!(closing(c, &id), Some((ClosingKind::Skipped, t)));
        assert!(c.closed_occurrence(&id).unwrap().paused.is_some());
        // The same skip twice isn't one device losing to the other.
        assert!(c.snapshot().reconciliations.is_empty());
        assert_eq!(
            c.state()
                .occurrences
                .values()
                .filter(|o| o.reminder_id == rid)
                .count(),
            2,
            "the 1st's and the 2nd's, one each"
        );
    }
}

#[test]
fn a_completion_made_elsewhere_beats_the_skip_a_pause_made() {
    let mut w = World::new(2);
    let rid = daily(w.dev(1), at("2026-10-01T06:00:00"));
    w.sync_all();
    let day1 = at("2026-10-01T07:00:00");
    w.dev(1).tick(day1).unwrap();
    w.dev(2).tick(day1 + 2).unwrap();
    w.sync_all();
    let id = occ(&rid, "2026-10-01T07:00:00");
    // Device 1 pauses (skipping the open one) as device 2 completes it.
    w.dev(1)
        .pause_reminder(
            &rid,
            Some(at("2026-10-05T00:00:00")),
            at("2026-10-01T07:30:00"),
        )
        .unwrap();
    w.dev(2).complete(&id, at("2026-10-01T07:31:00")).unwrap();
    w.sync_all();
    for i in 1..=2 {
        let c = w.dev(i);
        assert_eq!(closing(c, &id).unwrap().0, ClosingKind::Completed);
        assert!(c.is_paused(&rid, at("2026-10-02T00:00:00")));
    }
    // Device 1 is told what its skip came to.
    assert_eq!(w.dev(1).snapshot().reconciliations.len(), 1);
}

#[test]
fn devices_pausing_and_resuming_a_list_out_of_touch_converge() {
    for resume_last in [true, false] {
        let mut w = World::new(2);
        let home = w
            .dev(1)
            .create_list("Home", None, at("2026-10-01T05:00:00"))
            .unwrap();
        w.sync_all();
        let t = at("2026-10-01T20:00:00");
        w.dev(1)
            .pause_list(&home, Some(at("2026-10-08T00:00:00")), t)
            .unwrap();
        w.dev(2)
            .pause_list(&home, Some(at("2026-10-09T00:00:00")), t + 5)
            .unwrap();
        w.sync_all();
        let p1 = w.dev(1).state_of(&home).unwrap().list_pause;
        let p2 = w.dev(2).state_of(&home).unwrap().list_pause;
        assert_eq!(p1, p2);
        assert_eq!(p1.unwrap().until, Some(at("2026-10-09T00:00:00")));
        let versions = w.dev(1).state_of(&home).unwrap().list_pauses.len();
        assert_eq!(versions, 2);

        let (a, b) = if resume_last {
            (t + 50, t + 100)
        } else {
            (t + 100, t + 50)
        };
        w.dev(1).resume_list(&home, b).unwrap();
        w.dev(2)
            .pause_list(&home, Some(at("2026-10-12T00:00:00")), a)
            .unwrap();
        w.sync_all();
        let p1 = w.dev(1).state_of(&home).unwrap().list_pause;
        let p2 = w.dev(2).state_of(&home).unwrap().list_pause;
        assert_eq!(p1, p2);
        // Whichever was made last by the clock stands, on both.
        let later_is_resume = resume_last;
        assert_eq!(p1.is_none(), later_is_resume);
    }
}

// ---- Formats ----

#[test]
fn pausing_is_format_ten_and_what_came_before_is_not() {
    assert_eq!(FORMAT_VERSION, 11);
    let pause = Pause {
        from: 1,
        until: None,
    };
    let hlc = Hlc::default();
    assert_eq!(
        Event::ReminderEdited {
            reminder_id: "r".into(),
            hlc: hlc.clone(),
            change: Change::Pause(Some(pause)),
        }
        .format(),
        10
    );
    assert_eq!(
        Event::ListPaused {
            hlc: hlc.clone(),
            pause: None
        }
        .format(),
        10
    );
    assert_eq!(
        Event::OccurrenceSkippedForPause {
            occurrence_id: "o".into(),
            reminder_id: "r".into(),
            scheduled_at: 1,
            skipped_at: 1,
            until: None,
            list: false,
        }
        .format(),
        10
    );
    // An ordinary edit stays readable by the apps that came before.
    assert_eq!(
        Event::ReminderEdited {
            reminder_id: "r".into(),
            hlc,
            change: Change::Priority(Priority::High),
        }
        .format(),
        3
    );
}

#[test]
fn what_a_device_wrote_for_a_pause_is_sent_as_format_ten() {
    let mut w = World::new(1);
    let rid = daily(w.dev(1), at("2026-10-01T06:00:00"));
    let personal = w.dev(1).personal_list_id().to_string();
    let day1 = w.dev(1).tick(at("2026-10-01T07:00:00")).unwrap();
    w.dev(1)
        .complete(&day1[0].occurrence_id, at("2026-10-01T07:05:00"))
        .unwrap();
    w.upload(1);
    w.dev(1)
        .pause_reminder(
            &rid,
            Some(at("2026-10-05T00:00:00")),
            at("2026-10-01T20:00:00"),
        )
        .unwrap();
    w.dev(1)
        .pause_list(
            &personal,
            Some(at("2026-10-05T00:00:00")),
            at("2026-10-01T20:00:01"),
        )
        .unwrap();
    w.dev(1).tick(at("2026-10-02T07:00:00")).unwrap();
    let formats: Vec<u32> = w
        .dev(1)
        .unsent()
        .unwrap()
        .iter()
        .map(|o| o.format)
        .collect();
    assert_eq!(formats.len(), 3);
    assert!(formats.iter().all(|f| *f == 10), "{formats:?}");
}

// ---- Event triggers ----

#[test]
fn the_core_says_whether_a_reminder_is_paused_for_event_triggers_to_ask() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let now = at("2026-10-01T20:00:00");
    let until = at("2026-10-03T00:00:00");
    assert!(!c.is_paused(&rid, now));
    c.pause_reminder(&rid, Some(until), now).unwrap();
    assert!(c.is_paused(&rid, now));
    assert!(c.is_paused(&rid, until - 1));
    assert!(!c.is_paused(&rid, until));
    assert!(!c.is_paused("nope", now));
}

// ---- A harness for several devices ----

/// What the server keeps of an event: `(event_id, device_id, format, payload)`.
type Logged = (String, String, u32, Vec<u8>);

/// Devices of one user, and the server's numbering of their events, one
/// stream per list.
struct World {
    devices: Vec<Core>,
    log: BTreeMap<String, Vec<Logged>>,
}

impl World {
    fn new(n: usize) -> World {
        let mut first = Core::open_in_memory().unwrap();
        first.join(USER, "1").unwrap();
        first.set_device_zone("UTC").unwrap();
        let personal = first.personal_list_id().to_string();
        let mut devices = vec![first];
        for i in 2..=n {
            let mut c = Core::open_in_memory().unwrap();
            c.join(USER, &i.to_string()).unwrap();
            c.set_device_zone("UTC").unwrap();
            c.use_personal_list(&personal).unwrap();
            devices.push(c);
        }
        World {
            devices,
            log: Default::default(),
        }
    }

    fn dev(&mut self, i: usize) -> &mut Core {
        &mut self.devices[i - 1]
    }

    fn upload(&mut self, i: usize) {
        let id = i.to_string();
        for o in self.devices[i - 1].unsent().unwrap() {
            let stream = self.log.entry(o.list_id.clone()).or_default();
            stream.push((o.event_id.clone(), id.clone(), o.format, o.payload.clone()));
            let seq = stream.len() as i64;
            self.devices[i - 1].mark_sent(&o.event_id, seq).unwrap();
        }
    }

    fn download(&mut self, i: usize) {
        for (list, stream) in self.log.clone() {
            for (n, (event_id, device, format, payload)) in stream.iter().enumerate() {
                self.devices[i - 1]
                    .receive(&list, n as i64 + 1, event_id, device, *format, payload)
                    .unwrap();
            }
        }
    }

    fn sync_all(&mut self) {
        for i in 1..=self.devices.len() {
            self.upload(i);
        }
        for i in 1..=self.devices.len() {
            self.download(i);
        }
    }

    fn sync_in_order(&mut self, order: &[usize]) {
        for &i in order {
            self.upload(i);
        }
        for i in 1..=self.devices.len() {
            self.download(i);
        }
    }
}
