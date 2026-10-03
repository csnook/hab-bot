//! The merge rules for one user with several devices, through the core's own
//! API: devices act without syncing, then the server numbers what they made.

use hab_core::{Change, ClosingKind, Core, EditReminder, Event, Hlc, Setting, State, StoredEvent};

const T0: i64 = 1_000_000;
const USER: &str = "u1";

/// Devices of one user and the server's numbering of their events.
struct World {
    devices: Vec<Core>,
    /// The server's stream: `(event_id, device_id, format, payload)`.
    log: Vec<(String, String, u32, Vec<u8>)>,
    list: String,
}

impl World {
    /// `n` devices, each already holding the same synced reminder.
    fn new(n: usize) -> World {
        let mut first = Core::open_in_memory().unwrap();
        first.join(USER, "1").unwrap();
        let list = first.personal_list_id().to_string();
        let mut devices = vec![first];
        for i in 2..=n {
            let mut c = Core::open_in_memory().unwrap();
            c.join(USER, &i.to_string()).unwrap();
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

    /// Device `i` sends what it has made and the server numbers it.
    fn upload(&mut self, i: usize) {
        let id = i.to_string();
        let out = self.devices[i - 1].unsent().unwrap();
        for o in out {
            self.log
                .push((o.event_id.clone(), id.clone(), o.format, o.payload.clone()));
            let seq = self.log.len() as i64;
            self.devices[i - 1].mark_sent(&o.event_id, seq).unwrap();
        }
    }

    /// Device `i` downloads everything the server has numbered.
    fn download(&mut self, i: usize) {
        let list = self.list.clone();
        for (n, (event_id, device, format, payload)) in self.log.clone().iter().enumerate() {
            self.devices[i - 1]
                .receive(&list, n as i64 + 1, event_id, device, *format, payload)
                .unwrap();
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

    /// Everyone syncs, in this order of uploading.
    fn sync_in_order(&mut self, order: &[usize]) {
        for &i in order {
            self.upload(i);
        }
        for i in 1..=self.devices.len() {
            self.download(i);
        }
    }
}

/// A world of `n` devices that all hold a reminder that has fired and is open.
fn open_occurrence(n: usize) -> (World, String, String) {
    let mut w = World::new(n);
    w.dev(1).name_device("Desktop", T0).unwrap();
    if n > 1 {
        w.dev(2).name_device("Phone", T0).unwrap();
    }
    let rid = w.dev(1).create_reminder("Bins", T0 + 10, T0).unwrap();
    w.sync_all();
    let fired = w.dev(1).tick(T0 + 10).unwrap();
    w.sync_all();
    (w, rid, fired[0].occurrence_id.clone())
}

fn closing(c: &Core, occ: &str) -> Option<(ClosingKind, i64)> {
    c.state().occurrences[occ]
        .closing
        .as_ref()
        .map(|c| (c.kind, c.at))
}

fn act(c: &mut Core, kind: Option<ClosingKind>, occ: &str, at: i64) {
    match kind {
        Some(ClosingKind::Completed) => c.complete(occ, at).unwrap(),
        Some(ClosingKind::Skipped) => c.skip(occ, Some("busy"), at).unwrap(),
        Some(ClosingKind::Missed) => c.mark_missed(occ, at).unwrap(),
        None => {}
    }
}

#[test]
fn two_devices_acting_on_one_occurrence_follow_the_rules_whichever_syncs_first() {
    use ClosingKind::*;
    // (device 1's action, device 2's action, what counts, who is told)
    // Device 1 acts at T0+100, device 2 at T0+200.
    let table: &[(ClosingKind, ClosingKind, ClosingKind, Option<usize>)] = &[
        (Completed, Completed, Completed, None),
        (Skipped, Skipped, Skipped, None),
        (Missed, Missed, Missed, None),
        (Completed, Skipped, Completed, Some(2)),
        (Skipped, Completed, Completed, Some(1)),
        (Completed, Missed, Completed, Some(2)),
        (Missed, Completed, Completed, Some(1)),
        (Skipped, Missed, Skipped, None),
        (Missed, Skipped, Skipped, None),
    ];
    for &(a, b, wins, told) in table {
        for order in [[1usize, 2], [2, 1]] {
            let (mut w, _, occ) = open_occurrence(2);
            act(w.dev(1), Some(a), &occ, T0 + 100);
            act(w.dev(2), Some(b), &occ, T0 + 200);
            w.sync_in_order(&order);
            let what = format!("{a:?} on 1, {b:?} on 2, device {} uploads first", order[0]);

            // Both devices agree on the outcome.
            let c1 = closing(w.dev(1), &occ).unwrap_or_else(|| panic!("closed: {what}"));
            let c2 = closing(w.dev(2), &occ).unwrap();
            assert_eq!(c1, c2, "{what}");
            assert_eq!(c1.0, wins, "{what}");
            // Matching actions merge and the first one counts.
            if a == b {
                let first = if order[0] == 1 { T0 + 100 } else { T0 + 200 };
                assert_eq!(c1.1, first, "{what}");
            }
            // Only the device whose action lost to a completion is told, on every device.
            for d in 1..=2 {
                let notices = w.dev(d).snapshot().reconciliations;
                match told {
                    None => assert!(notices.is_empty(), "{what}"),
                    Some(loser) => {
                        assert_eq!(notices.len(), 1, "{what}");
                        assert_eq!(notices[0].device_id, loser.to_string(), "{what}");
                    }
                }
            }
        }
    }
}

#[test]
fn the_banner_says_which_device_lost_and_what_counts() {
    let (mut w, _, occ) = open_occurrence(2);
    w.dev(1).complete(&occ, T0 + 100).unwrap();
    w.dev(2).skip(&occ, None, T0 + 200).unwrap();
    w.sync_all();
    for d in 1..=2 {
        let n = w.dev(d).snapshot().reconciliations;
        assert_eq!(
            n[0].text,
            "Your Phone skipped \u{201c}Bins\u{201d}. It counts as completed."
        );
        assert_eq!(n[0].device_name.as_deref(), Some("Phone"));
    }
    // The completion is kept: who and when.
    assert_eq!(
        w.dev(1).state().occurrences[&occ].completed(),
        Some((USER.to_string(), T0 + 100))
    );
    // Dismissing hides it on that device only.
    let id = w.dev(2).snapshot().reconciliations[0].id.clone();
    w.dev(2).dismiss_notice(&id).unwrap();
    assert!(w.dev(2).snapshot().reconciliations.is_empty());
    assert_eq!(w.dev(1).snapshot().reconciliations.len(), 1);
    // Nothing is shown again once reconciled and the state is rebuilt.
    w.sync_all();
    assert!(w.dev(2).snapshot().reconciliations.is_empty());
}

#[test]
fn a_banner_waits_for_the_devices_to_meet_and_a_lone_action_has_none() {
    let (mut w, _, occ) = open_occurrence(2);
    w.dev(2).skip(&occ, None, T0 + 200).unwrap();
    w.sync_all();
    assert!(w.dev(1).snapshot().reconciliations.is_empty());
    assert!(w.dev(2).snapshot().reconciliations.is_empty());
    assert_eq!(closing(w.dev(1), &occ).unwrap().0, ClosingKind::Skipped);
}

#[test]
fn closing_beats_snoozing_and_acknowledging_whichever_arrives_first() {
    for close in [
        ClosingKind::Completed,
        ClosingKind::Skipped,
        ClosingKind::Missed,
    ] {
        for order in [[1usize, 2], [2, 1]] {
            let (mut w, _, occ) = open_occurrence(2);
            act(w.dev(1), Some(close), &occ, T0 + 100);
            w.dev(2).snooze(&occ, T0 + 900, T0 + 90).unwrap();
            w.dev(2).acknowledge(&occ, T0 + 95).unwrap();
            w.sync_in_order(&order);
            for d in 1..=2 {
                let o = &w.dev(d).state().occurrences[&occ];
                assert_eq!(o.closing.as_ref().unwrap().kind, close);
                assert_eq!((o.snoozed_until, o.acknowledged), (None, false));
                assert!(w.dev(d).snapshot().due.is_empty());
                // Not a disagreement between closings, so nobody is told.
                assert!(w.dev(d).snapshot().reconciliations.is_empty());
            }
        }
    }
}

#[test]
fn snoozing_and_acknowledging_apply_on_every_device_while_open() {
    let (mut w, _, occ) = open_occurrence(2);
    w.dev(1).snooze(&occ, T0 + 900, T0 + 20).unwrap();
    w.dev(2).acknowledge(&occ, T0 + 21).unwrap();
    w.sync_all();
    for d in 1..=2 {
        let due = w.dev(d).snapshot().due;
        assert_eq!(due[0].snoozed_until, Some(T0 + 900));
        assert!(due[0].acknowledged);
    }
}

#[test]
fn firings_of_the_same_instance_on_two_devices_merge_into_one_occurrence() {
    let mut w = World::new(2);
    let rid = w.dev(1).create_reminder("Bins", T0 + 10, T0).unwrap();
    w.sync_all();
    // Both fire it offline, a little apart.
    let a = w.dev(1).tick(T0 + 10).unwrap();
    let b = w.dev(2).tick(T0 + 13).unwrap();
    assert_eq!(a[0].occurrence_id, b[0].occurrence_id);
    w.sync_in_order(&[2, 1]);
    for d in 1..=2 {
        let s = w.dev(d).state();
        assert_eq!(s.occurrences.len(), 1);
        assert_eq!(
            s.occurrences[&format!("{rid}@{}", T0 + 10)].fired_at,
            T0 + 13
        );
        assert_eq!(w.dev(d).snapshot().due.len(), 1);
    }
}

#[test]
fn a_reminder_fired_on_two_devices_after_an_edit_of_its_time_is_still_one_occurrence() {
    let mut w = World::new(2);
    let rid = w.dev(1).create_reminder("Bins", T0 + 10, T0).unwrap();
    w.sync_all();
    // Device 1 moves it earlier and fires it; device 2 hasn't heard and fires
    // it at the old time.
    w.dev(1)
        .edit_reminder(
            &rid,
            EditReminder {
                fire_at: Some(T0 + 5),
                ..Default::default()
            },
            T0 + 1,
        )
        .unwrap();
    w.dev(1).tick(T0 + 5).unwrap();
    let theirs = w.dev(2).tick(T0 + 10).unwrap();
    w.dev(2)
        .complete(&theirs[0].occurrence_id, T0 + 11)
        .unwrap();
    w.sync_all();
    for d in 1..=2 {
        let s = w.dev(d).state();
        assert_eq!(s.occurrences.len(), 1, "one open occurrence per reminder");
        // The completion on the other id counts for the one occurrence.
        assert!(s.occurrences.values().next().unwrap().completed().is_some());
        assert!(w.dev(d).snapshot().due.is_empty());
    }
}

fn edit(
    c: &mut Core,
    rid: &str,
    title: Option<&str>,
    fire: Option<i64>,
    note: Option<&str>,
    at: i64,
) {
    c.edit_reminder(
        rid,
        EditReminder {
            title: title.map(str::to_string),
            fire_at: fire,
            note: note.map(str::to_string),
            ..Default::default()
        },
        at,
    )
    .unwrap();
}

fn settings(c: &Core, rid: &str) -> (String, i64, String) {
    let r = &c.state().reminders[rid];
    (r.title.clone(), r.fire_at, r.note.clone())
}

#[test]
fn the_setting_changed_last_wins_whichever_device_syncs_first() {
    for order in [[1usize, 2], [2, 1]] {
        let mut w = World::new(2);
        let rid = w.dev(1).create_reminder("Bins", T0 + 100, T0).unwrap();
        w.sync_all();
        edit(w.dev(1), &rid, Some("Recycling"), None, None, T0 + 10);
        edit(w.dev(2), &rid, Some("Compost"), None, None, T0 + 20);
        w.sync_in_order(&order);
        for d in 1..=2 {
            assert_eq!(settings(w.dev(d), &rid).0, "Compost", "order {order:?}");
        }
    }
}

#[test]
fn the_clock_decides_not_the_order_the_server_received_them_in() {
    // Device 2's edit is later but reaches the server first, then device 1's
    // earlier edit arrives: still device 2's wins.
    let mut w = World::new(2);
    let rid = w.dev(1).create_reminder("Bins", T0 + 100, T0).unwrap();
    w.sync_all();
    edit(w.dev(1), &rid, None, None, Some("early"), T0 + 10);
    edit(w.dev(2), &rid, None, None, Some("late"), T0 + 20);
    w.sync_in_order(&[2, 1]);
    assert_eq!(settings(w.dev(1), &rid).2, "late");
    assert_eq!(settings(w.dev(2), &rid).2, "late");
}

#[test]
fn a_device_whose_clock_is_behind_still_overrides_what_it_has_seen() {
    let mut w = World::new(2);
    let rid = w.dev(1).create_reminder("Bins", T0 + 100, T0).unwrap();
    w.sync_all();
    edit(w.dev(1), &rid, Some("Recycling"), None, None, T0 + 500);
    w.sync_all();
    // Device 2 has seen that, and edits with a clock five minutes behind.
    edit(w.dev(2), &rid, Some("Compost"), None, None, T0 + 200);
    w.sync_all();
    assert_eq!(settings(w.dev(1), &rid).0, "Compost");
}

#[test]
fn changes_to_different_settings_both_survive() {
    let mut w = World::new(2);
    let rid = w.dev(1).create_reminder("Bins", T0 + 100, T0).unwrap();
    w.sync_all();
    edit(w.dev(1), &rid, Some("Recycling"), None, None, T0 + 30);
    edit(
        w.dev(2),
        &rid,
        None,
        Some(T0 + 400),
        Some("Blue bin"),
        T0 + 20,
    );
    w.sync_in_order(&[1, 2]);
    for d in 1..=2 {
        assert_eq!(
            settings(w.dev(d), &rid),
            ("Recycling".to_string(), T0 + 400, "Blue bin".to_string())
        );
    }
}

#[test]
fn notes_do_not_merge_the_later_edit_replaces_the_earlier_whole() {
    let mut w = World::new(2);
    let rid = w.dev(1).create_reminder("Bins", T0 + 100, T0).unwrap();
    w.sync_all();
    edit(
        w.dev(1),
        &rid,
        None,
        None,
        Some("Blue bin out front"),
        T0 + 10,
    );
    edit(w.dev(2), &rid, None, None, Some("Take the keys"), T0 + 20);
    w.sync_all();
    for d in 1..=2 {
        assert_eq!(settings(w.dev(d), &rid).2, "Take the keys");
    }
}

#[test]
fn both_values_stay_in_the_history_and_the_loser_can_be_restored() {
    let mut w = World::new(2);
    let rid = w.dev(1).create_reminder("Bins", T0 + 100, T0).unwrap();
    w.sync_all();
    edit(w.dev(1), &rid, Some("Recycling"), None, None, T0 + 10);
    edit(w.dev(2), &rid, Some("Compost"), None, None, T0 + 20);
    w.sync_all();

    let h = w.dev(1).state().history(&rid, Setting::Title);
    let values: Vec<_> = h.iter().map(|v| v.change.clone()).collect();
    assert_eq!(
        values,
        vec![
            Change::Title("Compost".into()),
            Change::Title("Recycling".into()),
            Change::Title("Bins".into())
        ]
    );
    assert_eq!(
        h.iter().map(|v| v.current).collect::<Vec<_>>(),
        [true, false, false]
    );
    assert_eq!(h[1].device_id, "1");
    assert_eq!(h[0].by, USER);

    // Restoring the loser is a new change that wins everywhere.
    let lost = h[1].event_id.clone();
    w.dev(1)
        .restore_setting(&rid, Setting::Title, &lost, T0 + 40)
        .unwrap();
    w.sync_all();
    for d in 1..=2 {
        assert_eq!(settings(w.dev(d), &rid).0, "Recycling");
        // And nothing was lost by restoring.
        assert_eq!(w.dev(d).state().history(&rid, Setting::Title).len(), 4);
    }
    assert!(w
        .dev(1)
        .restore_setting(&rid, Setting::Title, "nope", T0 + 41)
        .is_err());
    assert!(w
        .dev(1)
        .restore_setting("nope", Setting::Title, &lost, T0 + 41)
        .is_err());
}

#[test]
fn editing_to_the_same_value_or_an_empty_title_changes_nothing_or_fails() {
    let mut w = World::new(1);
    let rid = w.dev(1).create_reminder("Bins", T0 + 100, T0).unwrap();
    let before = w.dev(1).unsent().unwrap().len();
    edit(
        w.dev(1),
        &rid,
        Some(" Bins "),
        Some(T0 + 100),
        Some(""),
        T0 + 1,
    );
    assert_eq!(w.dev(1).unsent().unwrap().len(), before);
    assert!(w
        .dev(1)
        .edit_reminder(
            &rid,
            EditReminder {
                title: Some("  ".into()),
                ..Default::default()
            },
            T0
        )
        .is_err());
    assert!(w
        .dev(1)
        .edit_reminder("nope", EditReminder::default(), T0)
        .is_err());
}

#[test]
fn an_edit_to_the_time_moves_when_the_reminder_fires() {
    let mut w = World::new(1);
    let rid = w.dev(1).create_reminder("Bins", T0 + 100, T0).unwrap();
    edit(w.dev(1), &rid, None, Some(T0 + 500), None, T0 + 1);
    assert!(w.dev(1).tick(T0 + 100).unwrap().is_empty());
    assert_eq!(w.dev(1).tick(T0 + 500).unwrap().len(), 1);
}

#[test]
fn a_clock_set_far_ahead_cannot_win_every_edit_forever() {
    let mut w = World::new(2);
    let rid = w.dev(1).create_reminder("Bins", T0 + 100, T0).unwrap();
    w.sync_all();
    // Device 1's clock claims to be a year ahead in the change itself.
    let wild = Hlc::next(T0 + 31_536_000, "1", &Hlc::default());
    let list = w.list.clone();
    let payload = serde_json::to_vec(&hab_core::Payload {
        author: USER.into(),
        recorded_at: T0 + 10,
        event: serde_json::to_value(Event::ReminderEdited {
            reminder_id: rid.clone(),
            hlc: wild,
            change: Change::Title("Wild".into()),
        })
        .unwrap(),
    })
    .unwrap();
    w.dev(2)
        .receive(&list, 99, "wild", "1", 1, &payload)
        .unwrap();
    assert_eq!(settings(w.dev(2), &rid).0, "Wild");
    // An honest edit made soon after (by the clock the server enforces) wins.
    edit(w.dev(2), &rid, Some("Honest"), None, None, T0 + 10 + 1_300);
    assert_eq!(settings(w.dev(2), &rid).0, "Honest");
}

// ---- Delivery in any order ----

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

/// Whatever order the same events reach a device in, the settings and the kind
/// of closing come out the same (which closing of a kind counts first depends
/// on the server's order, which every device shares).
#[test]
fn the_outcome_does_not_depend_on_the_order_events_arrive_in() {
    let edit_ev = |c: u32, ch: Change| Event::ReminderEdited {
        reminder_id: "r".into(),
        hlc: Hlc {
            wall_ms: 5_000,
            counter: c,
            node: "x".into(),
        },
        change: ch,
    };
    let events = [
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
        stored(2, "1", 5, edit_ev(0, Change::Title("A".into()))),
        stored(3, "2", 5, edit_ev(2, Change::Title("B".into()))),
        stored(4, "2", 5, edit_ev(1, Change::Note("n".into()))),
        stored(
            5,
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
            6,
            "1",
            6,
            Event::OccurrenceSkipped {
                occurrence_id: "r@9".into(),
                skipped_at: 6,
                note: None,
            },
        ),
        stored(
            7,
            "2",
            7,
            Event::OccurrenceCompleted {
                occurrence_id: "r@9".into(),
                completed_at: 7,
            },
        ),
        stored(
            8,
            "2",
            7,
            Event::OccurrenceSnoozed {
                occurrence_id: "r@9".into(),
                until: 99,
            },
        ),
    ];
    // The opening comes before what acts on it, as it does on every device.
    let outcome = |order: &[usize]| {
        let mut s = State::default();
        for &i in order {
            s.apply(&events[i]);
        }
        let r = &s.reminders["r"];
        let o = &s.occurrences["r@9"];
        (
            r.title.clone(),
            r.note.clone(),
            r.fire_at,
            o.closing.as_ref().map(|c| c.kind),
            (o.snoozed_until, o.acknowledged),
            s.reconciliations.len(),
        )
    };
    let reference = outcome(&[0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(
        reference,
        (
            "B".to_string(),
            "n".to_string(),
            9,
            Some(ClosingKind::Completed),
            (None, false),
            1
        )
    );
    // Edits and the later actions can arrive in any order among themselves.
    for p in permutations(&[1, 2, 3]) {
        for q in permutations(&[5, 6, 7]) {
            let mut order = vec![0];
            order.extend(&p);
            order.push(4);
            order.extend(&q);
            assert_eq!(outcome(&order), reference, "{order:?}");
        }
    }
}

#[test]
fn an_edit_that_arrives_before_the_reminder_still_applies() {
    let events = [
        stored(
            1,
            "2",
            5,
            Event::ReminderEdited {
                reminder_id: "r".into(),
                hlc: Hlc::next(5, "2", &Hlc::default()),
                change: Change::Title("Edited".into()),
            },
        ),
        stored(
            2,
            "1",
            1,
            Event::ReminderCreated {
                reminder_id: "r".into(),
                title: "Bins".into(),
                fire_at: 9,
            },
        ),
    ];
    let mut s = State::default();
    for e in &events {
        s.apply(e);
    }
    assert_eq!(s.reminders["r"].title, "Edited");
}
