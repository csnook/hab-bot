//! Keeping the lists tidy (#54): which overdue occurrences fold into the
//! older-quiet row, and Skip all… and Undo all on it. Time is passed in, so
//! every test says what the clock reads.

use hab_core::{folds, ClosingKind, Core, EditReminder, Error, Priority, UndoOutcome, FOLD_AFTER};

const USER: &str = "u1";
const T0: i64 = 1_000_000;
const DAY: i64 = 86_400;

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

/// A one-off reminder of `priority` that fires at `at`; returns its occurrence.
fn fired(c: &mut Core, title: &str, priority: Priority, at: i64) -> String {
    let id = c.create_reminder(title, at, at - 60).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            priority: Some(priority),
            ..Default::default()
        },
        at - 60,
    )
    .unwrap();
    c.tick(at).unwrap()[0].occurrence_id.clone()
}

fn overdue_at(c: &Core, occ: &str, now: i64) -> i64 {
    c.inbox(now)
        .overdue
        .iter()
        .find(|d| d.occurrence_id == occ)
        .expect("overdue")
        .overdue_at
}

#[test]
fn minimum_and_low_fold_after_a_week_overdue_and_nothing_above_does() {
    let mut c = core();
    let occs: Vec<(Priority, String)> = [
        Priority::Minimum,
        Priority::Low,
        Priority::Medium,
        Priority::High,
        Priority::Maximum,
    ]
    .into_iter()
    .map(|p| (p, fired(&mut c, &format!("{p:?}"), p, T0)))
    .collect();
    // Everything has been overdue for a month.
    let now = T0 + 30 * DAY;
    let inbox = c.inbox(now);
    assert_eq!(inbox.overdue.len(), 5);
    let folded: Vec<&String> = inbox.folded.iter().collect();
    assert_eq!(folded.len(), 2);
    for (p, id) in &occs {
        assert_eq!(
            inbox.folded.contains(id),
            *p <= Priority::Low,
            "{p:?} folds only if Minimum or Low"
        );
    }
}

#[test]
fn it_folds_only_once_it_has_been_overdue_for_more_than_seven_days() {
    let mut c = core();
    let occ = fired(&mut c, "Dust", Priority::Low, T0);
    let od = overdue_at(&c, &occ, T0 + 30 * DAY);
    assert!(c.inbox(od + FOLD_AFTER - 1).folded.is_empty());
    // Exactly a week is not "more than".
    assert!(c.inbox(od + FOLD_AFTER).folded.is_empty());
    assert_eq!(c.inbox(od + FOLD_AFTER + 1).folded, vec![occ]);
    // Due, not yet overdue, never folds.
    assert!(!folds(Priority::Low, od, od - 1));
}

#[test]
fn the_priority_that_counts_is_the_current_one_overrides_included() {
    let mut c = core();
    let occ = fired(&mut c, "Dust", Priority::Low, T0);
    let now = T0 + 30 * DAY;
    assert_eq!(c.inbox(now).folded, vec![occ.clone()]);
    let rid = c.state().occurrences[&occ].reminder_id.clone();
    c.edit_reminder(
        &rid,
        EditReminder {
            priority: Some(Priority::High),
            ..Default::default()
        },
        now,
    )
    .unwrap();
    assert!(c.inbox(now).folded.is_empty());
}

#[test]
fn what_is_paused_or_closed_is_not_in_the_row() {
    let mut c = core();
    let a = fired(&mut c, "A", Priority::Low, T0);
    let b = fired(&mut c, "B", Priority::Low, T0);
    let now = T0 + 30 * DAY;
    c.skip(&a, None, now - 1).unwrap();
    let rb = c.state().occurrences[&b].reminder_id.clone();
    c.pause_reminder(&rb, None, now).unwrap();
    assert!(c.inbox(now + 1).folded.is_empty());
}

#[test]
fn folding_leaves_the_sections_alone_for_the_tray_and_the_alerter() {
    let mut c = core();
    let occ = fired(&mut c, "Dust", Priority::Low, T0);
    let now = T0 + 30 * DAY;
    let inbox = c.inbox(now);
    // Still in Overdue, still open: folding is only how the window draws it.
    assert!(inbox.overdue.iter().any(|d| d.occurrence_id == occ));
    assert!(c.state().occurrences[&occ].is_open());
}

#[test]
fn skip_all_records_one_undoable_skip_each_and_leaves_the_rest() {
    let mut c = core();
    let low = fired(&mut c, "Low", Priority::Low, T0);
    let min = fired(&mut c, "Min", Priority::Minimum, T0);
    let med = fired(&mut c, "Med", Priority::Medium, T0);
    let max = fired(&mut c, "Max", Priority::Maximum, T0);
    let recent = fired(&mut c, "Recent", Priority::Low, T0 + 29 * DAY);
    let now = T0 + 30 * DAY;
    let before = c.recent_skip_notes(10);
    // Even if asked to, nothing that doesn't fold is skipped.
    let ids = [
        low.clone(),
        min.clone(),
        med.clone(),
        max.clone(),
        recent.clone(),
        "nope".into(),
    ];
    let skipped = c.skip_folded(&ids, now).unwrap();
    assert_eq!(skipped, vec![low.clone(), min.clone()]);
    for id in [&low, &min] {
        let o = &c.state().occurrences[id];
        assert_eq!(o.closing.as_ref().unwrap().kind, ClosingKind::Skipped);
        // A skip like any other: it can be undone and its history says so.
        assert!(c.closed_occurrence(id).unwrap().can_undo);
    }
    for id in [&med, &max, &recent] {
        assert!(c.state().occurrences[id].is_open());
    }
    // No note, so the suggestions for skip notes are unchanged.
    assert_eq!(c.recent_skip_notes(10), before);
    // Doing it again finds nothing left.
    assert!(c.skip_folded(&ids, now).unwrap().is_empty());
}

#[test]
fn undo_all_reopens_them_one_event_each() {
    let mut c = core();
    let a = fired(&mut c, "A", Priority::Low, T0);
    let b = fired(&mut c, "B", Priority::Minimum, T0);
    let now = T0 + 30 * DAY;
    let skipped = c.skip_folded(&[a.clone(), b.clone()], now).unwrap();
    let undone = c.undo_all(&skipped, now + 5).unwrap();
    assert_eq!(undone.len(), 2);
    assert!(undone.iter().all(|(_, o)| *o == UndoOutcome::Reopened));
    for id in [&a, &b] {
        assert!(c.state().occurrences[id].is_open());
    }
    // Undone ones fold again, being just as old.
    assert_eq!(c.inbox(now + 6).folded.len(), 2);
}

#[test]
fn undo_all_skips_over_what_is_no_longer_closed_or_was_missed() {
    let mut c = core();
    let a = fired(&mut c, "A", Priority::Low, T0);
    let b = fired(&mut c, "B", Priority::Low, T0);
    let now = T0 + 30 * DAY;
    c.skip_folded(&[a.clone(), b.clone()], now).unwrap();
    c.undo(&a, now + 1).unwrap();
    let undone = c.undo_all(&[a.clone(), b.clone()], now + 2).unwrap();
    assert_eq!(undone.iter().map(|(i, _)| i).collect::<Vec<_>>(), vec![&b]);
    // `a` was undone already, so a second undo finds nothing closed.
    assert!(matches!(c.undo(&a, now + 3), Err(Error::NotClosed(_))));
    assert!(matches!(c.undo("nope", now + 3), Err(Error::NotClosed(_))));
}

// ---- Several devices ----

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
}

/// Two devices, each with the same three old Low occurrences open.
fn old_quiet() -> (World, Vec<String>) {
    let mut w = World::new(2);
    w.dev(1).name_device("Desktop", T0).unwrap();
    w.dev(2).name_device("Phone", T0).unwrap();
    let mut occs = Vec::new();
    for t in ["A", "B", "C"] {
        occs.push(fired(w.dev(1), t, Priority::Low, T0));
    }
    w.sync_all();
    (w, occs)
}

#[test]
fn skip_all_on_one_device_skips_on_both() {
    let (mut w, occs) = old_quiet();
    let now = T0 + 30 * DAY;
    assert_eq!(w.dev(2).inbox(now).folded.len(), 3);
    let skipped = w.dev(2).skip_folded(&occs, now).unwrap();
    assert_eq!(skipped.len(), 3);
    w.sync_all();
    for d in 1..=2 {
        for id in &occs {
            let o = &w.dev(d).state().occurrences[id];
            assert_eq!(o.closing.as_ref().unwrap().kind, ClosingKind::Skipped);
        }
        assert!(w.dev(d).inbox(now + 1).overdue.is_empty());
    }
    // Undo all on the other device takes them all back, everywhere.
    let skipped = w.dev(1).skip_folded(&occs, now + 2).unwrap();
    assert!(skipped.is_empty());
    let undone = w.dev(1).undo_all(&occs, now + 3).unwrap();
    assert_eq!(undone.len(), 3);
    w.sync_all();
    for d in 1..=2 {
        assert_eq!(w.dev(d).inbox(now + 4).folded.len(), 3, "device {d}");
        assert_eq!(w.dev(d).snapshot().reconciliations.len(), 0, "device {d}");
    }
}

#[test]
fn a_completion_on_another_device_beats_a_skip_all_in_either_order() {
    for order in [[1usize, 2], [2, 1]] {
        let (mut w, occs) = old_quiet();
        let now = T0 + 30 * DAY;
        // The phone completes one of them while the desktop skips all three.
        w.dev(2).complete(&occs[1], now + 5).unwrap();
        let skipped = w.dev(1).skip_folded(&occs, now).unwrap();
        assert_eq!(skipped.len(), 3);
        w.sync_in_order(&order);
        for d in 1..=2 {
            let kinds: Vec<ClosingKind> = occs
                .iter()
                .map(|id| {
                    w.dev(d).state().occurrences[id]
                        .closing
                        .as_ref()
                        .unwrap()
                        .kind
                })
                .collect();
            assert_eq!(
                kinds,
                vec![
                    ClosingKind::Skipped,
                    ClosingKind::Completed,
                    ClosingKind::Skipped
                ],
                "device {d}, order {order:?}"
            );
        }
        // The user is told on both devices that the skip lost, once, for the
        // one occurrence, and not for the two that were skipped.
        for d in 1..=2 {
            let notices = w.dev(d).snapshot().reconciliations;
            assert_eq!(notices.len(), 1, "device {d}, order {order:?}");
            assert_eq!(notices[0].occurrence_id, occs[1]);
        }
    }
}

#[test]
fn undo_all_leaves_what_a_completion_elsewhere_won_and_takes_back_the_skips() {
    let (mut w, occs) = old_quiet();
    let now = T0 + 30 * DAY;
    w.dev(1).skip_folded(&occs, now).unwrap();
    w.dev(2).complete(&occs[1], now + 5).unwrap();
    w.sync_all();
    let undone = w.dev(1).undo_all(&occs, now + 10).unwrap();
    assert_eq!(undone.len(), 2);
    w.sync_all();
    for d in 1..=2 {
        let kinds: Vec<Option<ClosingKind>> = occs
            .iter()
            .map(|id| {
                w.dev(d).state().occurrences[id]
                    .closing
                    .as_ref()
                    .map(|c| c.kind)
            })
            .collect();
        assert_eq!(
            kinds,
            vec![None, Some(ClosingKind::Completed), None],
            "device {d}"
        );
    }
}

#[test]
fn undo_all_leaves_a_skip_that_another_device_corrected() {
    let (mut w, occs) = old_quiet();
    let now = T0 + 30 * DAY;
    w.dev(1).skip_folded(&occs, now).unwrap();
    w.sync_all();
    // The phone corrects one skip to completed, then the desktop undoes all.
    w.dev(2)
        .correct(
            &occs[0],
            hab_core::Correction::Completed,
            now - 10,
            None,
            now + 1,
        )
        .unwrap();
    w.sync_all();
    let undone = w.dev(1).undo_all(&occs, now + 2).unwrap();
    // The corrected one is no longer a skip, so Undo all leaves it.
    assert_eq!(undone.len(), 2);
    w.sync_all();
    for d in 1..=2 {
        assert!(
            !w.dev(d).state().occurrences[&occs[0]].is_open(),
            "device {d}"
        );
        assert!(
            w.dev(d).state().occurrences[&occs[1]].is_open(),
            "device {d}"
        );
        assert!(
            w.dev(d).state().occurrences[&occs[2]].is_open(),
            "device {d}"
        );
    }
}
