//! Acting on occurrences (#45): completing at a time or early, skipping
//! ahead, undo and correct, and what happens when devices disagree around
//! them. Time is passed in, so every test says what the clock reads.

use hab_core::{
    ClosingKind, Core, Correction, Countdown, CountdownUnit, EditReminder, Error, Event,
    HistoryWhat, Outcome, Schedule, State, StoredEvent, UndoOutcome, FORMAT_VERSION,
};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

const USER: &str = "u1";
const T0: i64 = 1_000_000;
const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

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

fn watering(c: &mut Core, last: i64) -> String {
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

fn closing(c: &Core, occ: &str) -> Option<(ClosingKind, i64)> {
    c.state().occurrences[occ]
        .closing
        .as_ref()
        .map(|c| (c.kind, c.at))
}

fn is_open(c: &Core, occ: &str) -> bool {
    c.state().occurrences[occ].is_open()
}

// ---- Completing at a time, and early ----

#[test]
fn done_at_a_time_before_the_firing_keeps_the_time_said_and_the_time_tapped() {
    let mut c = core();
    let fire = at("2026-10-01T07:00:00");
    let rid = c.create_reminder("Medicine", fire, fire - DAY).unwrap();
    let occ = c.tick(fire).unwrap()[0].occurrence_id.clone();
    assert!(occ.starts_with(&rid));
    // "Took it at 6:55", tapped at 7:20.
    let tapped = fire + 20 * 60;
    let said = fire - 5 * 60;
    c.complete_at(&occ, said, tapped).unwrap();
    assert_eq!(closing(&c, &occ), Some((ClosingKind::Completed, said)));
    let v = c.closed_occurrence(&occ).unwrap();
    assert_eq!(v.history.len(), 1);
    assert_eq!(v.history[0].at, Some(said));
    assert_eq!(v.history[0].tapped_at, tapped);
    // The server's receive time is added once it has numbered the event.
    assert_eq!(v.history[0].received_at, None);
    c.set_received_at(&v.history[0].event_id, tapped + 2)
        .unwrap();
    let v = c.closed_occurrence(&occ).unwrap();
    assert_eq!(v.history[0].received_at, Some(tapped + 2));
    // A time still to come can't be said.
    let again = c.create_reminder("Again", fire, fire - DAY).unwrap();
    let occ2 = c.tick(fire).unwrap();
    let occ2 = occ2.iter().find(|f| f.reminder_id == again).unwrap();
    assert!(matches!(
        c.complete_at(&occ2.occurrence_id, tapped + 1, tapped),
        Err(Error::InTheFuture)
    ));
}

#[test]
fn completing_early_closes_the_next_scheduled_occurrence_which_never_fires() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let day1 = c.tick(at("2026-10-01T07:00:00")).unwrap();
    // While day one is open, completing the next early is refused.
    let night = at("2026-10-01T22:00:00");
    assert!(matches!(
        c.complete_expected(&rid, night, night),
        Err(Error::StillOpen(_))
    ));
    assert!(!c.offers_early(&rid, night));
    c.complete(&day1[0].occurrence_id, night).unwrap();
    assert!(c.offers_early(&rid, night));
    // Done early at 22:00 for tomorrow's 07:00.
    let day2 = at("2026-10-02T07:00:00");
    let occ = c.complete_expected(&rid, night, night).unwrap();
    assert_eq!(occ, format!("{rid}@{day2}"));
    assert_eq!(closing(&c, &occ), Some((ClosingKind::Completed, night)));
    // It never fires: nothing at 07:00 tomorrow, nothing open, and the next
    // expected is the day after.
    assert!(c.tick(day2).unwrap().is_empty());
    assert!(c.inbox(day2).overdue.is_empty() && c.inbox(day2).due.is_empty());
    assert_eq!(c.next_fire_at(), Some(at("2026-10-03T07:00:00")));
    assert_eq!(c.tick(at("2026-10-03T07:00:00")).unwrap().len(), 1);
}

#[test]
fn skipping_ahead_takes_a_note_and_the_note_is_offered_again() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let now = at("2026-10-01T09:00:00");
    // Nothing fired yet today: tomorrow's is not next, today's has passed...
    // so skip the next expected one, whichever it is.
    let occ = c.skip_expected(&rid, Some("  away  "), now).unwrap();
    let o = &c.state().occurrences[&occ];
    let closing = o.closing.as_ref().unwrap();
    assert_eq!(closing.kind, ClosingKind::Skipped);
    assert_eq!(closing.note.as_deref(), Some("away"));
    assert!(c.tick(o.scheduled_at).unwrap().is_empty());
    // Notes offered, most recent first, each once.
    let second = c.skip_expected(&rid, Some("travelling"), now + 10).unwrap();
    assert_ne!(second, occ);
    c.skip_expected(&rid, Some("away"), now + 20).unwrap();
    assert_eq!(c.recent_skip_notes(5), vec!["away", "travelling"]);
    assert_eq!(c.recent_skip_notes(1), vec!["away"]);
}

#[test]
fn a_one_off_can_be_completed_early_and_a_fired_one_has_nothing_expected() {
    let mut c = core();
    let rid = c.create_reminder("Call", T0 + DAY, T0).unwrap();
    assert!(c.offers_early(&rid, T0));
    let occ = c.complete_expected(&rid, T0 + 5, T0 + 5).unwrap();
    assert!(c.state().is_finished(&rid));
    assert!(c.tick(T0 + DAY).unwrap().is_empty());
    assert!(c.snapshot().upcoming.is_empty());
    assert!(!c.offers_early(&rid, T0 + 10));
    assert!(matches!(
        c.complete_expected(&rid, T0 + 10, T0 + 10),
        Err(Error::NotExpected(_))
    ));
    assert_eq!(closing(&c, &occ).unwrap().0, ClosingKind::Completed);
}

#[test]
fn a_countdown_restarts_from_completing_early() {
    let mut c = core();
    let t0 = at("2026-10-01T08:00:00");
    let rid = watering(&mut c, t0);
    let pending = t0 + 3 * DAY;
    let watered = t0 + DAY;
    c.complete_expected(&rid, watered, watered).unwrap();
    assert_eq!(c.next_fire_at(), Some(watered + 3 * DAY));
    assert!(c.tick(pending).unwrap().is_empty());
}

// ---- Undo ----

#[test]
fn undo_reopens_a_completed_or_skipped_occurrence_that_would_still_be_open() {
    for skip in [false, true] {
        let mut c = core();
        let rid = c.create_reminder("Bins", T0, T0 - 5).unwrap();
        let occ = c.tick(T0).unwrap()[0].occurrence_id.clone();
        if skip {
            c.skip(&occ, Some("later"), T0 + 10).unwrap();
        } else {
            c.complete(&occ, T0 + 10).unwrap();
        }
        assert!(c.state().is_finished(&rid));
        assert_eq!(c.undo(&occ, T0 + 20).unwrap(), UndoOutcome::Reopened);
        assert!(is_open(&c, &occ));
        assert!(!c.state().is_finished(&rid));
        assert_eq!(c.snapshot().due.len(), 1);
        // The history keeps the undone closing, marked.
        let h = c.state().history_of(&occ);
        assert_eq!(h.len(), 2);
        assert!(h[0].superseded);
        assert_eq!(h[1].what, HistoryWhat::Reopened);
        // It can be acted on again.
        c.complete(&occ, T0 + 30).unwrap();
        assert_eq!(closing(&c, &occ), Some((ClosingKind::Completed, T0 + 30)));
    }
}

#[test]
fn an_undone_occurrence_alerts_again() {
    use hab_core::{Alerter, Command};
    let mut c = core();
    c.create_reminder("Bins", T0, T0 - 5).unwrap();
    let occ = c.tick(T0).unwrap()[0].occurrence_id.clone();
    let mut a = Alerter::new();
    let shown = |cmds: &[Command]| cmds.iter().any(|c| matches!(c, Command::Show(_)));
    assert!(shown(&a.pass(&mut c, T0 + 1, false).unwrap().commands));
    c.complete(&occ, T0 + 5).unwrap();
    a.pass(&mut c, T0 + 6, false).unwrap();
    c.undo(&occ, T0 + 10).unwrap();
    assert!(shown(&a.pass(&mut c, T0 + 11, false).unwrap().commands));
}

#[test]
fn a_miss_is_corrected_not_undone() {
    let mut c = core();
    c.create_reminder("Bins", T0, T0 - 5).unwrap();
    let occ = c.tick(T0).unwrap()[0].occurrence_id.clone();
    c.mark_missed(&occ, T0 + 100).unwrap();
    assert!(matches!(c.undo(&occ, T0 + 110), Err(Error::CantUndoMiss)));
    assert!(matches!(
        c.undo("nonsense", T0 + 110),
        Err(Error::NotClosed(_))
    ));
    // An open one isn't closed, so there is nothing to undo.
    let other = c.create_reminder("Open", T0, T0 - 5).unwrap();
    let open = c.tick(T0 + 120).unwrap();
    let open = open.iter().find(|f| f.reminder_id == other).unwrap();
    assert!(matches!(
        c.undo(&open.occurrence_id, T0 + 130),
        Err(Error::NotClosed(_))
    ));
}

#[test]
fn undo_after_the_expiry_leaves_it_missed_as_of_the_expiry() {
    let mut c = core();
    let rid = c.create_reminder("Pills", T0, T0 - 5).unwrap();
    c.edit_reminder(
        &rid,
        EditReminder {
            expiry: Some(vec![hab_core::Delay::After(HOUR)]),
            ..Default::default()
        },
        T0 - 4,
    )
    .unwrap();
    let occ = c.tick(T0).unwrap()[0].occurrence_id.clone();
    c.complete(&occ, T0 + 60).unwrap();
    // Undone within the hour: reopened, and it still expires on time.
    assert_eq!(c.undo(&occ, T0 + 120).unwrap(), UndoOutcome::Reopened);
    c.complete(&occ, T0 + 130).unwrap();
    // Undone after the expiry: missed, as of when it expired.
    let late = T0 + 2 * HOUR;
    assert_eq!(
        c.undo(&occ, late).unwrap(),
        UndoOutcome::Missed { at: T0 + HOUR }
    );
    assert_eq!(closing(&c, &occ), Some((ClosingKind::Missed, T0 + HOUR)));
    assert!(!is_open(&c, &occ));
}

#[test]
fn undo_after_a_newer_occurrence_fired_leaves_it_missed() {
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let day1 = c.tick(at("2026-10-01T07:00:00")).unwrap()[0]
        .occurrence_id
        .clone();
    c.complete(&day1, at("2026-10-01T07:05:00")).unwrap();
    let day2 = c.tick(at("2026-10-02T07:00:00")).unwrap()[0]
        .occurrence_id
        .clone();
    let outcome = c.undo(&day1, at("2026-10-02T08:00:00")).unwrap();
    assert_eq!(
        outcome,
        UndoOutcome::Missed {
            at: at("2026-10-02T07:00:00")
        }
    );
    // One open occurrence only (ADR 0001), and it is the newer.
    assert!(!is_open(&c, &day1));
    assert!(is_open(&c, &day2));
    assert_eq!(
        c.state()
            .occurrences
            .values()
            .filter(|o| o.reminder_id == rid && o.is_open())
            .count(),
        1
    );
}

#[test]
fn undoing_a_countdown_closing_moves_the_countdown_back() {
    let mut c = core();
    let t0 = at("2026-10-01T08:00:00");
    let rid = watering(&mut c, t0);
    let fires = t0 + 3 * DAY;
    let occ = c.tick(fires).unwrap()[0].occurrence_id.clone();
    let done = fires + HOUR;
    c.complete(&occ, done).unwrap();
    assert_eq!(c.next_fire_at(), Some(done + 3 * DAY));
    // Reopened: the countdown waits for it to close again.
    assert_eq!(c.undo(&occ, done + 60).unwrap(), UndoOutcome::Reopened);
    assert_eq!(c.next_fire_at(), None);
    assert_eq!(c.snapshot().countdowns[0].next_at, None);
    // With an expiry that has passed it becomes missed, and counts from then.
    c.complete(&occ, done + 120).unwrap();
    c.edit_reminder(
        &rid,
        EditReminder {
            expiry: Some(vec![hab_core::Delay::After(2 * HOUR)]),
            ..Default::default()
        },
        done + 130,
    )
    .unwrap();
    let much_later = fires + DAY;
    assert!(matches!(
        c.undo(&occ, much_later).unwrap(),
        UndoOutcome::Missed { .. }
    ));
    let missed_at = closing(&c, &occ).unwrap().1;
    assert_eq!(missed_at, fires + 2 * HOUR);
    // Moved back from done + 3 days to the miss + 3 days.
    assert_eq!(c.next_fire_at(), Some(missed_at + 3 * DAY));
    assert!(missed_at + 3 * DAY < done + 3 * DAY + DAY);
}

#[test]
fn undoing_an_early_closing_makes_it_expected_again_and_it_fires_at_its_time() {
    // A countdown.
    let mut c = core();
    let t0 = at("2026-10-01T08:00:00");
    let rid = watering(&mut c, t0);
    let pending = t0 + 3 * DAY;
    let now = t0 + DAY;
    let occ = c.complete_expected(&rid, now, now).unwrap();
    assert_eq!(c.next_fire_at(), Some(now + 3 * DAY));
    assert_eq!(c.undo(&occ, now + 60).unwrap(), UndoOutcome::Expected);
    assert!(!is_open(&c, &occ));
    assert!(c.state().occurrences[&occ].unfired);
    // The countdown is back where it was, and the occurrence is expected.
    assert_eq!(c.next_fire_at(), Some(pending));
    assert_eq!(c.snapshot().countdowns[0].next_at, Some(pending));
    assert_eq!(c.expected(now + 61, pending).len(), 1);
    // It fires at its time, as the same occurrence.
    let fired = c.tick(pending).unwrap();
    assert_eq!(fired[0].occurrence_id, occ);
    assert!(is_open(&c, &occ));

    // A schedule.
    let mut c = core();
    let rid = daily(&mut c, at("2026-10-01T06:00:00"));
    let night = at("2026-10-01T22:00:00");
    let occ = c.complete_expected(&rid, night, night).unwrap();
    assert_eq!(c.undo(&occ, night + 5).unwrap(), UndoOutcome::Expected);
    assert!(c
        .expected(night + 6, at("2026-10-02T23:00:00"))
        .iter()
        .any(|e| e.scheduled_at == at("2026-10-02T07:00:00")));
    let first = c.tick(at("2026-10-02T07:00:00")).unwrap();
    assert!(!first.is_empty());
    // A one-off.
    let mut c = core();
    let rid = c.create_reminder("Call", T0 + DAY, T0).unwrap();
    let occ = c.complete_expected(&rid, T0 + 5, T0 + 5).unwrap();
    assert_eq!(c.undo(&occ, T0 + 6).unwrap(), UndoOutcome::Expected);
    assert_eq!(c.snapshot().upcoming.len(), 1);
    assert_eq!(c.tick(T0 + DAY).unwrap()[0].occurrence_id, occ);
}

// ---- Correct ----

#[test]
fn correcting_changes_a_closing_and_keeps_the_original() {
    let mut c = core();
    c.create_reminder("Bins", T0, T0 - 5).unwrap();
    let occ = c.tick(T0).unwrap()[0].occurrence_id.clone();
    c.skip(&occ, Some("oops"), T0 + 10).unwrap();
    // It was in fact done, at 9:40-ish.
    c.correct(&occ, Correction::Completed, T0 + 5, None, T0 + 100)
        .unwrap();
    assert_eq!(closing(&c, &occ), Some((ClosingKind::Completed, T0 + 5)));
    let v = c.closed_occurrence(&occ).unwrap();
    assert!(v.corrected);
    assert_eq!(v.history.len(), 2);
    assert_eq!(v.history[0].what, HistoryWhat::Skipped);
    assert!(v.history[0].superseded);
    assert_eq!(v.history[0].note.as_deref(), Some("oops"));
    assert_eq!(v.history[1].what, HistoryWhat::Completed);
    assert!(!v.history[1].superseded);
    // A correction can be corrected again; every step is kept.
    c.correct(&occ, Correction::Skipped, T0 + 6, Some("no"), T0 + 200)
        .unwrap();
    assert_eq!(closing(&c, &occ), Some((ClosingKind::Skipped, T0 + 6)));
    assert_eq!(c.closed_occurrence(&occ).unwrap().history.len(), 3);
    // A time still to come can't be said.
    assert!(matches!(
        c.correct(&occ, Correction::Completed, T0 + 300, None, T0 + 200),
        Err(Error::InTheFuture)
    ));
    // Undoing a corrected closing reopens the occurrence, not one step back.
    c.undo(&occ, T0 + 210).unwrap();
    assert!(is_open(&c, &occ));
    assert_eq!(c.closed_occurrence(&occ).map(|v| v.history.len()), None);
    assert_eq!(c.state().history_of(&occ).len(), 4);
}

#[test]
fn a_miss_corrected_to_completed_counts_as_done_late() {
    let mut c = core();
    let rid = c.create_reminder("Pills", T0, T0 - 5).unwrap();
    c.edit_reminder(
        &rid,
        EditReminder {
            expiry: Some(vec![hab_core::Delay::After(HOUR)]),
            ..Default::default()
        },
        T0 - 4,
    )
    .unwrap();
    let occ = c.tick(T0).unwrap()[0].occurrence_id.clone();
    c.tick(T0 + 2 * HOUR).unwrap();
    assert_eq!(closing(&c, &occ), Some((ClosingKind::Missed, T0 + HOUR)));
    assert_eq!(c.closed_occurrence(&occ).unwrap().outcome, Outcome::Missed);
    assert!(!c.closed_occurrence(&occ).unwrap().can_undo);
    // Actually taken at 9:40, well within what was due.
    let said = T0 + 60;
    c.correct(&occ, Correction::Completed, said, None, T0 + 3 * HOUR)
        .unwrap();
    let v = c.closed_occurrence(&occ).unwrap();
    assert_eq!(
        v.outcome,
        Outcome::DoneLate,
        "a miss corrected is done late"
    );
    assert!(v.history[0].superseded);
    assert_eq!(v.history[0].what, HistoryWhat::Missed);
    // The same, corrected again to a different time, is still late.
    c.correct(&occ, Correction::Completed, said + 1, None, T0 + 4 * HOUR)
        .unwrap();
    assert_eq!(
        c.closed_occurrence(&occ).unwrap().outcome,
        Outcome::DoneLate
    );
    // Whereas a completion while due is on time.
    let rid2 = c.create_reminder("Bins", T0 + 10_000, T0).unwrap();
    let occ2 = c.tick(T0 + 10_000).unwrap();
    let occ2 = &occ2
        .iter()
        .find(|f| f.reminder_id == rid2)
        .unwrap()
        .occurrence_id;
    c.complete(occ2, T0 + 10_001).unwrap();
    assert_eq!(
        c.closed_occurrence(occ2).unwrap().outcome,
        Outcome::DoneOnTime
    );
}

#[test]
fn correcting_a_countdown_closing_moves_the_next_firing() {
    let mut c = core();
    let t0 = at("2026-10-01T08:00:00");
    let rid = watering(&mut c, t0);
    let fires = t0 + 3 * DAY;
    let occ = c.tick(fires).unwrap()[0].occurrence_id.clone();
    c.complete(&occ, fires + 10 * HOUR).unwrap();
    assert_eq!(c.next_fire_at(), Some(fires + 10 * HOUR + 3 * DAY));
    // It was really done two hours after it fired.
    c.correct(
        &occ,
        Correction::Completed,
        fires + 2 * HOUR,
        None,
        fires + 11 * HOUR,
    )
    .unwrap();
    assert_eq!(c.next_fire_at(), Some(fires + 2 * HOUR + 3 * DAY));
    let _ = rid;
}

#[test]
fn undo_and_correct_events_need_a_newer_format() {
    let ev = Event::OccurrenceUndone {
        occurrence_id: "o".into(),
        replaces: vec![],
        outcome: UndoOutcome::Reopened,
    };
    assert_eq!(ev.format(), 9);
    assert_eq!(FORMAT_VERSION, 11);
    let ev = Event::OccurrenceCorrected {
        occurrence_id: "o".into(),
        replaces: vec![],
        kind: Correction::Skipped,
        at: 1,
        note: None,
    };
    assert_eq!(ev.format(), 9);
    // Both survive the wire.
    let json = serde_json::to_string(&Event::OccurrenceUndone {
        occurrence_id: "o".into(),
        replaces: vec!["a".into()],
        outcome: UndoOutcome::Missed { at: 5 },
    })
    .unwrap();
    assert!(serde_json::from_str::<Event>(&json).is_ok());
}

// ---- Devices that disagree ----

struct World {
    devices: Vec<Core>,
    log: Vec<(String, String, u32, Vec<u8>)>,
    list: String,
}

impl World {
    fn new(n: usize) -> World {
        let mut first = Core::open_in_memory().unwrap();
        first.join(USER, "1").unwrap();
        first.set_device_zone("UTC").unwrap();
        let list = first.personal_list_id().to_string();
        let mut devices = vec![first];
        for i in 2..=n {
            let mut c = Core::open_in_memory().unwrap();
            c.join(USER, &i.to_string()).unwrap();
            c.set_device_zone("UTC").unwrap();
            c.use_personal_list(&list).unwrap();
            devices.push(c);
        }
        World {
            devices,
            log: Vec::new(),
            list,
        }
    }

    fn dev(&mut self, i: usize) -> &mut Core {
        &mut self.devices[i - 1]
    }

    fn upload(&mut self, i: usize) {
        let id = i.to_string();
        for o in self.devices[i - 1].unsent().unwrap() {
            self.log
                .push((o.event_id.clone(), id.clone(), o.format, o.payload.clone()));
            let seq = self.log.len() as i64;
            self.devices[i - 1].mark_sent(&o.event_id, seq).unwrap();
        }
    }

    fn download(&mut self, i: usize) {
        let list = self.list.clone();
        for (n, (event_id, device, format, payload)) in self.log.clone().iter().enumerate() {
            self.devices[i - 1]
                .receive(&list, n as i64 + 1, event_id, device, *format, payload)
                .unwrap();
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

    fn sync_all(&mut self) {
        self.sync_in_order(&(1..=self.devices.len()).collect::<Vec<_>>());
    }

    fn notices(&mut self, i: usize) -> usize {
        self.dev(i).snapshot().reconciliations.len()
    }
}

fn open_pair() -> (World, String) {
    let mut w = World::new(2);
    w.dev(1).name_device("Desktop", T0).unwrap();
    w.dev(2).name_device("Phone", T0).unwrap();
    w.dev(1).create_reminder("Bins", T0 + 10, T0).unwrap();
    w.sync_all();
    let occ = w.dev(1).tick(T0 + 10).unwrap()[0].occurrence_id.clone();
    w.sync_all();
    (w, occ)
}

#[test]
fn an_undo_after_the_devices_met_takes_back_what_both_had_done() {
    for order in [[1usize, 2], [2, 1]] {
        let (mut w, occ) = open_pair();
        w.dev(1).complete(&occ, T0 + 100).unwrap();
        w.dev(2).skip(&occ, None, T0 + 110).unwrap();
        w.sync_in_order(&order);
        // A completion beat the skip, and the phone was told.
        assert_eq!(w.notices(2), 1);
        // The desktop saw both, and undoes the lot.
        assert_eq!(
            w.dev(1).undo(&occ, T0 + 200).unwrap(),
            UndoOutcome::Reopened
        );
        w.sync_in_order(&[1, 2]);
        for d in 1..=2 {
            assert!(is_open(w.dev(d), &occ), "device {d}, order {order:?}");
            // An undo is not read as the devices disagreeing.
            assert_eq!(w.notices(d), 0, "device {d}");
        }
    }
}

#[test]
fn an_undo_does_not_take_back_what_another_device_did_meanwhile() {
    for order in [[1usize, 2], [2, 1]] {
        let (mut w, occ) = open_pair();
        // The desktop completes and undoes before it hears of the phone's skip.
        w.dev(1).complete(&occ, T0 + 100).unwrap();
        w.dev(1).undo(&occ, T0 + 120).unwrap();
        w.dev(2).skip(&occ, Some("away"), T0 + 110).unwrap();
        w.sync_in_order(&order);
        for d in 1..=2 {
            // The phone's skip stands, and nobody is told of a disagreement:
            // the completion had been deliberately undone.
            assert_eq!(
                closing(w.dev(d), &occ),
                Some((ClosingKind::Skipped, T0 + 110)),
                "device {d}, order {order:?}"
            );
            assert_eq!(w.notices(d), 0, "device {d}");
        }
    }
}

#[test]
fn a_correction_is_not_a_disagreement_and_the_original_is_kept_on_every_device() {
    for order in [[1usize, 2], [2, 1]] {
        let (mut w, occ) = open_pair();
        w.dev(1).skip(&occ, None, T0 + 100).unwrap();
        w.sync_all();
        w.dev(2)
            .correct(&occ, Correction::Completed, T0 + 90, None, T0 + 200)
            .unwrap();
        w.sync_in_order(&order);
        for d in 1..=2 {
            assert_eq!(
                closing(w.dev(d), &occ),
                Some((ClosingKind::Completed, T0 + 90))
            );
            // A skip lost to a completion, but the completion was a
            // deliberate correction of it: nobody is told.
            assert_eq!(w.notices(d), 0, "device {d}");
            let h = w.dev(d).closed_occurrence(&occ).unwrap().history;
            assert_eq!(h.len(), 2);
            assert_eq!(h.iter().filter(|e| e.superseded).count(), 1);
        }
    }
}

#[test]
fn devices_correcting_the_same_closing_differently_follow_the_usual_rules() {
    for order in [[1usize, 2], [2, 1]] {
        let (mut w, occ) = open_pair();
        w.dev(1).mark_missed(&occ, T0 + 100).unwrap();
        w.sync_all();
        w.dev(1)
            .correct(&occ, Correction::Skipped, T0 + 150, None, T0 + 300)
            .unwrap();
        w.dev(2)
            .correct(&occ, Correction::Completed, T0 + 140, None, T0 + 310)
            .unwrap();
        w.sync_in_order(&order);
        for d in 1..=2 {
            // A completion beats a skip, and the device that skipped is told.
            assert_eq!(
                closing(w.dev(d), &occ),
                Some((ClosingKind::Completed, T0 + 140))
            );
            assert_eq!(w.dev(d).snapshot().reconciliations.len(), 1);
            assert_eq!(w.dev(d).snapshot().reconciliations[0].device_id, "1");
        }
    }
}

#[test]
fn a_correction_stands_over_a_concurrent_undo_and_two_undos_merge() {
    for order in [[1usize, 2], [2, 1]] {
        let (mut w, occ) = open_pair();
        w.dev(1).complete(&occ, T0 + 100).unwrap();
        w.sync_all();
        w.dev(1).undo(&occ, T0 + 200).unwrap();
        w.dev(2)
            .correct(&occ, Correction::Completed, T0 + 50, None, T0 + 210)
            .unwrap();
        w.sync_in_order(&order);
        for d in 1..=2 {
            assert_eq!(
                closing(w.dev(d), &occ),
                Some((ClosingKind::Completed, T0 + 50)),
                "device {d}, order {order:?}"
            );
        }
        // Two devices undoing the same thing is one undo.
        let (mut w, occ) = open_pair();
        w.dev(1).complete(&occ, T0 + 100).unwrap();
        w.sync_all();
        w.dev(1).undo(&occ, T0 + 200).unwrap();
        w.dev(2).undo(&occ, T0 + 201).unwrap();
        w.sync_in_order(&order);
        for d in 1..=2 {
            assert!(is_open(w.dev(d), &occ));
            assert_eq!(w.notices(d), 0);
        }
    }
}

#[test]
fn an_undo_made_before_hearing_of_a_newer_firing_ends_missed_on_every_device() {
    let mut w = World::new(2);
    let t = at("2026-10-01T06:00:00");
    let rid = daily(w.dev(1), t);
    w.sync_all();
    let day1 = at("2026-10-01T07:00:00");
    let day2 = at("2026-10-02T07:00:00");
    let occ1 = w.dev(1).tick(day1).unwrap()[0].occurrence_id.clone();
    w.sync_all();
    w.dev(1).complete(&occ1, day1 + 60).unwrap();
    w.sync_all();
    // The phone, awake at day two, fires the new instance; the desktop,
    // which hasn't heard, undoes yesterday's and sees it reopen.
    let occ2 = w.dev(2).tick(day2).unwrap()[0].occurrence_id.clone();
    assert_eq!(
        w.dev(1).undo(&occ1, day2 - 3600).unwrap(),
        UndoOutcome::Reopened
    );
    let _ = rid;
    w.sync_in_order(&[2, 1]);
    for d in 1..=2 {
        // Only the newer is open (ADR 0001): yesterday's is missed.
        let s = w.dev(d).state();
        assert!(s.occurrences[&occ2].is_open());
        let c = s.occurrences[&occ1].closing.as_ref().expect("closed");
        assert_eq!((c.kind, c.at), (ClosingKind::Missed, day2));
    }
}

// ---- Order independence ----

fn stored(n: usize, device: &str, at: i64, event: Event) -> StoredEvent {
    StoredEvent {
        list_id: "l".into(),
        seq: Some(n as i64),
        event_id: format!("e{n}"),
        device_id: device.into(),
        author: USER.into(),
        recorded_at: at,
        event,
    }
}

fn permutations(items: &[usize]) -> Vec<Vec<usize>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, x);
            out.push(p);
        }
    }
    out
}

type Digest = (Option<(ClosingKind, i64)>, bool, usize, Vec<(String, bool)>);

/// What an occurrence comes to, for comparing orders: how it is closed, how
/// many notices, and which entries of the history were taken back.
fn digest(s: &State, occ: &str) -> Digest {
    let o = &s.occurrences[occ];
    (
        o.closing.as_ref().map(|c| (c.kind, c.at)),
        o.is_open(),
        s.reconciliations.len(),
        s.history_of(occ)
            .into_iter()
            .map(|h| (h.event_id, h.superseded))
            .collect(),
    )
}

#[test]
fn closings_corrections_and_undos_settle_the_same_in_any_order() {
    // Two devices act, a third corrects what it saw, a fourth undoes the
    // correction. The opening comes first, as it does on every device.
    let events = vec![
        stored(
            1,
            "1",
            1,
            Event::ReminderCreated {
                reminder_id: "r".into(),
                title: "Bins".into(),
                fire_at: 9,
            },
        ),
        stored(
            2,
            "1",
            5,
            Event::OccurrenceOpened {
                occurrence_id: "r@9".into(),
                reminder_id: "r".into(),
                scheduled_at: 9,
                fired_at: 9,
            },
        ),
        stored(
            3,
            "1",
            10,
            Event::OccurrenceCompleted {
                occurrence_id: "r@9".into(),
                completed_at: 10,
            },
        ),
        stored(
            4,
            "2",
            11,
            Event::OccurrenceSkipped {
                occurrence_id: "r@9".into(),
                skipped_at: 11,
                note: None,
            },
        ),
        stored(
            5,
            "1",
            20,
            Event::OccurrenceCorrected {
                occurrence_id: "r@9".into(),
                replaces: vec!["e3".into(), "e4".into()],
                kind: Correction::Skipped,
                at: 15,
                note: Some("n".into()),
            },
        ),
        stored(
            6,
            "2",
            30,
            Event::OccurrenceUndone {
                occurrence_id: "r@9".into(),
                replaces: vec!["e5".into(), "e3".into(), "e4".into()],
                outcome: UndoOutcome::Reopened,
            },
        ),
        stored(
            7,
            "2",
            31,
            Event::OccurrenceUndone {
                occurrence_id: "r@9".into(),
                replaces: vec!["e5".into(), "e3".into(), "e4".into()],
                outcome: UndoOutcome::Missed { at: 31 },
            },
        ),
    ];
    let run = |order: &[usize]| {
        let mut s = State::default();
        s.apply(&events[0]);
        s.apply(&events[1]);
        for &i in order {
            s.apply(&events[i]);
        }
        digest(&s, "r@9")
    };
    let reference = run(&[2, 3, 4, 5, 6]);
    // Two undos of the corrected closing: one reopened it, one left it
    // missed. A closing beats a reopening, whichever came first.
    assert_eq!(reference.0, Some((ClosingKind::Missed, 31)));
    for order in permutations(&[2, 3, 4, 5, 6]) {
        assert_eq!(run(&order), reference, "order {order:?}");
    }
    // Without the undos, the correction stands.
    let run2 = |order: &[usize]| {
        let mut s = State::default();
        s.apply(&events[0]);
        s.apply(&events[1]);
        for &i in order {
            s.apply(&events[i]);
        }
        digest(&s, "r@9")
    };
    let reference = run2(&[2, 3, 4]);
    assert_eq!(reference.0, Some((ClosingKind::Skipped, 15)));
    assert_eq!(reference.2, 0, "a correction is not a disagreement");
    for order in permutations(&[2, 3, 4]) {
        assert_eq!(run2(&order), reference, "order {order:?}");
    }
    // Duplicates change nothing.
    let mut s = State::default();
    for e in &events {
        s.apply(e);
        s.apply(e);
    }
    assert_eq!(digest(&s, "r@9").0, Some((ClosingKind::Missed, 31)));
}
