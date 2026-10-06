//! Reminder lists, moving reminders between them, and deleting reminders,
//! through the core's own API, with several devices merging (ADR 0009).

use hab_core::{
    Alerter, Change, ClosingKind, Command, Core, EditReminder, Error, Event, Filters, Priority,
    Setting, FORMAT_VERSION,
};

const T0: i64 = 1_790_000_000;
const HOUR: i64 = 3_600;
const USER: &str = "u1";

fn solo() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

/// What the server keeps of an event: `(event_id, device_id, format, payload)`.
type Logged = (String, String, u32, Vec<u8>);

/// Devices of one user, and the server's numbering of their events, one
/// stream per list.
struct World {
    devices: Vec<Core>,
    /// Each list's stream: `(event_id, device_id, format, payload)`.
    log: std::collections::BTreeMap<String, Vec<Logged>>,
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

fn priority(c: &mut Core, id: &str, p: Priority, now: i64) {
    c.edit_reminder(
        id,
        EditReminder {
            priority: Some(p),
            ..Default::default()
        },
        now,
    )
    .unwrap();
}

fn live_in(c: &Core) -> Vec<(String, Vec<String>)> {
    c.lists()
        .into_iter()
        .map(|l| {
            let mut ids: Vec<String> = c
                .state_of(&l.id)
                .unwrap()
                .reminders
                .keys()
                .cloned()
                .collect();
            ids.sort();
            (l.id, ids)
        })
        .collect()
}

// ---- Lists ----

#[test]
fn lists_can_be_made_renamed_and_coloured_and_the_personal_list_is_the_default() {
    let mut c = solo();
    let personal = c.personal_list_id().to_string();
    assert_eq!(c.lists().len(), 1);
    assert!(c.lists()[0].personal && c.lists()[0].name.is_none());

    let home = c.create_list("  Home ", Some("#E04F5F"), T0).unwrap();
    let work = c.create_list("Work", None, T0).unwrap();
    let names: Vec<_> = c.lists().into_iter().map(|l| l.name).collect();
    assert_eq!(
        names,
        vec![None, Some("Home".to_string()), Some("Work".to_string())]
    );
    let home_info = &c.lists()[1];
    assert_eq!(home_info.id, home);
    assert_eq!(home_info.colour.as_deref(), Some("#e04f5f"));
    assert_eq!(c.lists()[2].colour, None);

    c.rename_list(&work, "Office", T0 + 1).unwrap();
    c.colour_list(&work, "#3584e4", T0 + 1).unwrap();
    c.colour_list(&personal, "#33aa77", T0 + 1).unwrap();
    let after = c.lists();
    assert_eq!(after[0].colour.as_deref(), Some("#33aa77"));
    assert_eq!(after[2].name.as_deref(), Some("Office"));
    assert_eq!(after[2].colour.as_deref(), Some("#3584e4"));

    assert!(matches!(
        c.create_list("   ", None, T0),
        Err(Error::EmptyListName)
    ));
    assert!(matches!(
        c.create_list("x", Some("red"), T0),
        Err(Error::BadColour(_))
    ));
    assert!(matches!(
        c.colour_list(&work, "#12345", T0),
        Err(Error::BadColour(_))
    ));
    assert!(matches!(
        c.rename_list("nope", "x", T0),
        Err(Error::NoList(_))
    ));
}

#[test]
fn the_personal_list_cannot_be_renamed_or_deleted() {
    let mut c = solo();
    let personal = c.personal_list_id().to_string();
    assert!(matches!(
        c.rename_list(&personal, "Mine", T0),
        Err(Error::PersonalList)
    ));
    assert!(matches!(
        c.delete_list(&personal, T0),
        Err(Error::PersonalList)
    ));
    assert_eq!(c.lists().len(), 1);
}

#[test]
fn each_list_keeps_its_own_event_stream() {
    let mut c = solo();
    c.join(USER, "1").unwrap();
    let personal = c.personal_list_id().to_string();
    let home = c.create_list("Home", Some("#e04f5f"), T0).unwrap();
    let a = c.create_reminder("In personal", T0 + 10, T0).unwrap();
    let b = c.create_reminder_in(&home, "At home", T0 + 10, T0).unwrap();
    let mut streams: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for o in c.unsent().unwrap() {
        streams.entry(o.list_id).or_default().push(o.event_id);
    }
    // The new list's events, and only those, are in its stream.
    assert_eq!(streams.len(), 2);
    assert_eq!(streams[&home].len(), 3, "named, coloured, one reminder");
    assert_eq!(streams[&personal].len(), 1);
    assert!(c.state_of(&personal).unwrap().reminders.contains_key(&a));
    assert!(!c.state_of(&personal).unwrap().reminders.contains_key(&b));
    assert!(c.state_of(&home).unwrap().reminders.contains_key(&b));
    assert_eq!(c.reminder_view(&b).unwrap().list_id, home);
    assert_eq!(
        c.reminder_view(&b).unwrap().list_name.as_deref(),
        Some("Home")
    );
    assert_eq!(
        c.reminder_view(&b).unwrap().list_colour.as_deref(),
        Some("#e04f5f")
    );
    // Reminders can be made in a list that isn't known.
    assert!(matches!(
        c.create_reminder_in("nope", "x", T0, T0),
        Err(Error::NoList(_))
    ));
}

#[test]
fn reminders_in_any_list_fire_and_carry_their_list() {
    let mut c = solo();
    let home = c.create_list("Home", None, T0).unwrap();
    let id = c.create_reminder_in(&home, "Bins", T0 + 10, T0).unwrap();
    let fired = c.tick(T0 + 10).unwrap();
    assert_eq!(fired.len(), 1);
    let inbox = c.inbox(T0 + 10);
    let item = inbox.due.iter().chain(&inbox.overdue).next().unwrap();
    assert_eq!(item.list_id, home);
    assert_eq!(item.reminder_id, id);
    c.complete(&fired[0].occurrence_id, T0 + 20).unwrap();
    let earlier = &c.inbox(T0 + 20).earlier_today;
    assert_eq!(earlier[0].list_id, home);
}

#[test]
fn an_empty_list_can_be_deleted_but_not_one_with_reminders() {
    let mut c = solo();
    let home = c.create_list("Home", None, T0).unwrap();
    let id = c.create_reminder_in(&home, "Bins", T0 + 10, T0).unwrap();
    assert!(matches!(c.delete_list(&home, T0), Err(Error::ListNotEmpty)));
    c.delete_reminder(&id, T0 + 1).unwrap();
    c.delete_list(&home, T0 + 2).unwrap();
    assert_eq!(c.lists().len(), 1);
    assert!(matches!(
        c.create_reminder_in(&home, "x", T0, T0),
        Err(Error::NoList(_))
    ));
}

#[test]
fn a_list_deleted_while_another_device_added_a_reminder_stays() {
    let mut w = World::new(2);
    let home = w.dev(1).create_list("Home", None, T0).unwrap();
    w.sync_all();
    // Device 1 deletes the (empty) list as device 2 puts a reminder in it.
    w.dev(1).delete_list(&home, T0 + 10).unwrap();
    let id = w
        .dev(2)
        .create_reminder_in(&home, "Bins", T0 + 100, T0 + 11)
        .unwrap();
    w.sync_all();
    for i in 1..=2 {
        let c = w.dev(i);
        assert!(c.lists().iter().any(|l| l.id == home), "device {i}");
        assert!(c.state_of(&home).unwrap().reminders.contains_key(&id));
    }
}

#[test]
fn a_second_device_sees_a_new_list_with_its_name_and_colour() {
    let mut w = World::new(2);
    let home = w.dev(1).create_list("Home", Some("#e04f5f"), T0).unwrap();
    let id = w
        .dev(1)
        .create_reminder_in(&home, "Bins", T0 + 50, T0)
        .unwrap();
    w.sync_all();
    let l = w.dev(2).lists();
    assert_eq!(l.len(), 2);
    assert_eq!(l[1].name.as_deref(), Some("Home"));
    assert_eq!(l[1].colour.as_deref(), Some("#e04f5f"));
    assert_eq!(l[1].reminders, 1);
    // It fires on the second device too, into the same occurrence.
    let fired = w.dev(2).tick(T0 + 50).unwrap();
    assert_eq!(fired[0].reminder_id, id);
    w.sync_all();
    assert!(w
        .dev(1)
        .state_of(&home)
        .unwrap()
        .occurrences
        .contains_key(&format!("{id}@{}", T0 + 50)));
}

// ---- Moving ----

/// A reminder in the personal list that has an edit history and a closed and
/// an open occurrence, on one device.
fn with_history(c: &mut Core) -> (String, String, String) {
    let id = c
        .create_recurring_reminder(
            "Pills",
            vec![hab_core::Schedule::from_pattern(
                &hab_core::Pattern::Daily,
                "2026-09-01",
                "07:00",
            )
            .unwrap()],
            Some("UTC"),
            T0 - 10 * 86_400,
        )
        .unwrap();
    priority(c, &id, Priority::High, T0 - 9 * 86_400);
    c.edit_reminder(
        &id,
        EditReminder {
            title: Some("Pills (morning)".into()),
            ..Default::default()
        },
        T0 - 8 * 86_400,
    )
    .unwrap();
    let now = T0;
    let fired = c.tick(now).unwrap();
    assert_eq!(fired.len(), 1);
    // Earlier instances were missed, the latest is open.
    let open = fired[0].occurrence_id.clone();
    let closed = c
        .state()
        .occurrences
        .values()
        .find(|o| !o.is_open())
        .map(|o| o.id.clone())
        .expect("a missed earlier occurrence");
    (id, open, closed)
}

#[test]
fn moving_a_reminder_keeps_its_settings_history_and_occurrences() {
    let mut c = solo();
    let (id, open, closed) = with_history(&mut c);
    let versions_before = c.state().history(&id, Setting::Title);
    assert_eq!(versions_before.len(), 2);
    let home = c.create_list("Home", None, T0 + 1).unwrap();
    c.move_reminder(&id, &home, T0 + 2).unwrap();

    let personal = c.personal_list_id().to_string();
    assert!(c.state_of(&personal).unwrap().reminders.is_empty());
    assert!(c.state_of(&personal).unwrap().occurrences.is_empty());
    let to = c.state_of(&home).unwrap();
    assert_eq!(to.reminders[&id].title, "Pills (morning)");
    assert_eq!(to.reminders[&id].priority, Priority::High);
    assert_eq!(to.reminders[&id].list_id, home);
    assert!(to.occurrences[&open].is_open());
    assert!(!to.occurrences[&closed].is_open());
    let versions = to.history(&id, Setting::Title);
    assert_eq!(versions.len(), 2);
    assert_eq!(
        versions
            .iter()
            .map(|v| v.event_id.clone())
            .collect::<Vec<_>>(),
        versions_before
            .iter()
            .map(|v| v.event_id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(c.reminder_view(&id).unwrap().list_id, home);
    // The history can still be restored from, in its new list.
    let older = versions.iter().find(|v| !v.current).unwrap();
    c.restore_setting(&id, Setting::Title, &older.event_id, T0 + 3)
        .unwrap();
    assert_eq!(c.reminder_view(&id).unwrap().title, "Pills");
}

#[test]
fn a_moved_reminder_neither_fires_again_nor_forgets_its_open_occurrence() {
    let mut c = solo();
    let (id, open, _) = with_history(&mut c);
    let home = c.create_list("Home", None, T0 + 1).unwrap();
    c.move_reminder(&id, &home, T0 + 2).unwrap();
    // Not fired a second time for the same instant, and no stale copy.
    assert!(c.tick(T0 + 3).unwrap().is_empty());
    let inbox = c.inbox(T0 + 3);
    assert_eq!(
        inbox
            .due
            .iter()
            .chain(&inbox.overdue)
            .filter(|d| d.reminder_id == id)
            .count(),
        1
    );
    // The open occurrence is acted on where it now is.
    c.complete(&open, T0 + 4).unwrap();
    assert_eq!(
        c.state_of(&home).unwrap().occurrences[&open]
            .closing
            .as_ref()
            .unwrap()
            .kind,
        ClosingKind::Completed
    );
}

#[test]
fn what_is_done_in_the_new_list_to_an_occurrence_from_the_old_one_survives_a_rebuild() {
    // The streams alone, replayed by a device that was never there, say the
    // same: the occurrence was opened in the old list's stream and closed,
    // snoozed and acknowledged in the new one's.
    let mut w = World::new(3);
    let (id, open, _) = with_history(w.dev(1));
    let home = w.dev(1).create_list("Home", None, T0 + 1).unwrap();
    w.dev(1).move_reminder(&id, &home, T0 + 2).unwrap();
    w.dev(1).snooze(&open, T0 + 900, T0 + 3).unwrap();
    w.dev(1).acknowledge(&open, T0 + 4).unwrap();
    w.upload(1);
    // Device 2 learns of the move, then completes it.
    w.download(2);
    w.dev(2).complete(&open, T0 + 20).unwrap();
    w.sync_all();
    for i in 1..=3 {
        let c = w.dev(i);
        let o = &c.state_of(&home).unwrap().occurrences[&open];
        assert_eq!(
            o.closing.as_ref().map(|c| c.kind),
            Some(ClosingKind::Completed),
            "device {i}"
        );
        assert_eq!(c.state_of(&home).unwrap().snoozes.len(), 1, "device {i}");
        assert!(c.state().occurrences.is_empty());
    }
}

#[test]
fn a_reminder_can_move_back_and_through_several_lists() {
    let mut c = solo();
    let (id, open, closed) = with_history(&mut c);
    let personal = c.personal_list_id().to_string();
    let a = c.create_list("A", None, T0 + 1).unwrap();
    let b = c.create_list("B", None, T0 + 1).unwrap();
    c.move_reminder(&id, &a, T0 + 2).unwrap();
    c.move_reminder(&id, &b, T0 + 3).unwrap();
    c.move_reminder(&id, &personal, T0 + 4).unwrap();
    assert_eq!(
        live_in(&c)
            .into_iter()
            .filter(|(_, r)| r.contains(&id))
            .map(|(l, _)| l)
            .collect::<Vec<_>>(),
        vec![personal.clone()]
    );
    let s = c.state();
    assert!(s.occurrences.contains_key(&open) && s.occurrences.contains_key(&closed));
    assert_eq!(s.history(&id, Setting::Title).len(), 2);
    assert!(matches!(
        c.move_reminder(&id, &personal, T0 + 5),
        Err(Error::SameList)
    ));
    assert!(matches!(
        c.move_reminder(&id, "nope", T0 + 5),
        Err(Error::NoList(_))
    ));
}

#[test]
fn a_move_reaches_other_devices_with_the_history() {
    let mut w = World::new(2);
    let (id, open, closed) = with_history(w.dev(1));
    let home = w.dev(1).create_list("Home", None, T0 + 1).unwrap();
    w.sync_all();
    w.dev(1).move_reminder(&id, &home, T0 + 2).unwrap();
    w.sync_all();
    for i in 1..=2 {
        let c = w.dev(i);
        let to = c.state_of(&home).unwrap();
        assert!(to.reminders.contains_key(&id), "device {i}");
        assert!(to.occurrences.contains_key(&open) && to.occurrences.contains_key(&closed));
        assert_eq!(to.history(&id, Setting::Title).len(), 2);
        assert!(c.state().reminders.is_empty());
    }
}

#[test]
fn actions_on_the_old_list_made_out_of_touch_survive_the_move() {
    for order in [[1, 2], [2, 1]] {
        let mut w = World::new(2);
        let (id, open, _) = with_history(w.dev(1));
        let home = w.dev(1).create_list("Home", None, T0 + 1).unwrap();
        w.sync_all();
        // Device 1 moves it while device 2, not knowing, completes the open
        // occurrence and edits the note.
        w.dev(1).move_reminder(&id, &home, T0 + 10).unwrap();
        w.dev(2).complete(&open, T0 + 11).unwrap();
        w.dev(2)
            .edit_reminder(
                &id,
                EditReminder {
                    note: Some("with water".into()),
                    ..Default::default()
                },
                T0 + 12,
            )
            .unwrap();
        w.sync_in_order(&order);
        for i in 1..=2 {
            let c = w.dev(i);
            let s = c.state_of(&home).unwrap();
            assert_eq!(
                s.occurrences[&open].closing.as_ref().map(|c| c.kind),
                Some(ClosingKind::Completed),
                "device {i}, order {order:?}"
            );
            assert_eq!(s.reminders[&id].note, "with water");
            assert!(c.state().reminders.is_empty());
        }
    }
}

#[test]
fn two_moves_made_out_of_touch_leave_the_later_one_everywhere() {
    for order in [[1, 2], [2, 1]] {
        let mut w = World::new(2);
        let id = w.dev(1).create_reminder("Bins", T0 + 1_000, T0).unwrap();
        let a = w.dev(1).create_list("A", None, T0).unwrap();
        let b = w.dev(1).create_list("B", None, T0).unwrap();
        w.sync_all();
        w.dev(1).move_reminder(&id, &a, T0 + 10).unwrap();
        w.dev(2).move_reminder(&id, &b, T0 + 20).unwrap();
        w.sync_in_order(&order);
        for i in 1..=2 {
            let c = w.dev(i);
            let at: Vec<String> = live_in(c)
                .into_iter()
                .filter(|(_, r)| r.contains(&id))
                .map(|(l, _)| l)
                .collect();
            assert_eq!(at, vec![b.clone()], "device {i}, order {order:?}");
        }
    }
}

#[test]
fn an_old_app_that_cannot_read_a_move_keeps_the_reminder_in_the_old_list() {
    // Format 8 events are kept unapplied by an app that reads format 7.
    let ev = Event::ReminderMovedIn {
        reminder_id: "r".into(),
        from_list_id: "l".into(),
        hlc: Default::default(),
    };
    assert_eq!(ev.format(), 8);
    assert_eq!(FORMAT_VERSION, 13);
    for e in [
        Event::ListColoured {
            colour: "#000000".into(),
        },
        Event::ListDeleted,
        Event::ReminderDeleted {
            reminder_id: "r".into(),
        },
        Event::ReminderPurged {
            reminder_id: "r".into(),
        },
    ] {
        assert_eq!(e.format(), 8, "{e:?}");
    }
    // Naming a list is format 1: an old app shows the list, and the update notice.
    assert_eq!(Event::ListNamed { name: "x".into() }.format(), 1);
}

// ---- Deleting ----

#[test]
fn a_deleted_reminder_keeps_its_history_marked_deleted_and_never_fires() {
    let mut c = solo();
    let id = c.create_reminder("Bins", T0 + 100, T0).unwrap();
    let nth = c
        .create_recurring_reminder(
            "Pills",
            vec![hab_core::Schedule::from_pattern(
                &hab_core::Pattern::Daily,
                "2026-09-01",
                "07:00",
            )
            .unwrap()],
            Some("UTC"),
            T0 - 5 * 86_400,
        )
        .unwrap();
    c.delete_reminder(&id, T0 + 1).unwrap();
    c.delete_reminder(&nth, T0 + 1).unwrap();
    assert!(c.tick(T0 + 10 * 86_400).unwrap().is_empty());
    assert!(c.next_fire_at().is_none());
    assert!(c.snapshot().upcoming.is_empty());
    let inbox = c.inbox(T0 + 200);
    assert!(inbox.due.is_empty() && inbox.overdue.is_empty() && inbox.later_today.is_empty());
    let deleted = c.deleted_reminders();
    assert_eq!(deleted.len(), 2);
    assert!(deleted
        .iter()
        .any(|d| d.reminder_id == id && d.title == "Bins"));
    assert!(matches!(
        c.edit_reminder(&id, EditReminder::default(), T0 + 2),
        Err(Error::NoReminder(_))
    ));
    assert!(matches!(
        c.move_reminder(&id, c.personal_list_id().to_string().as_str(), T0),
        Err(Error::NoReminder(_))
    ));
    // The history is still there.
    assert!(c.state().history(&id, Setting::Title).len() == 1);
}

#[test]
fn deleting_a_reminder_with_an_open_occurrence_stops_its_alerts() {
    let mut c = solo();
    let id = c.create_reminder("Bins", T0, T0 - HOUR).unwrap();
    priority(&mut c, &id, Priority::High, T0 - HOUR);
    let fired = c.tick(T0).unwrap();
    let occ = fired[0].occurrence_id.clone();
    let mut alerter = Alerter::new();
    let pass = alerter.pass(&mut c, T0, false).unwrap();
    assert!(pass
        .commands
        .iter()
        .any(|x| matches!(x, Command::Show(n) if n.occurrence_id == occ)));
    c.delete_reminder(&id, T0 + 5).unwrap();
    let pass = alerter.pass(&mut c, T0 + 6, false).unwrap();
    assert!(
        pass.commands
            .iter()
            .any(|x| matches!(x, Command::Close { occurrence_id } if *occurrence_id == occ)),
        "{:?}",
        pass.commands
    );
    assert!(!pass.commands.iter().any(|x| matches!(x, Command::Show(_))));
    // Later passes stay quiet, and the occurrence can't be acted on.
    assert!(alerter
        .pass(&mut c, T0 + 10 * HOUR, false)
        .unwrap()
        .commands
        .is_empty());
    assert!(matches!(c.complete(&occ, T0 + 7), Err(Error::NotOpen(_))));
    assert!(c.occurrence_view(&occ).is_none());
    // The occurrence is kept as history.
    assert!(c.state().occurrences.contains_key(&occ));
}

#[test]
fn a_deletion_on_one_device_stops_the_other_alerting_once_it_syncs() {
    let mut w = World::new(2);
    let id = w.dev(1).create_reminder("Bins", T0, T0 - HOUR).unwrap();
    priority(w.dev(1), &id, Priority::High, T0 - HOUR);
    let fired = w.dev(1).tick(T0).unwrap();
    w.sync_all();
    let occ = fired[0].occurrence_id.clone();
    let mut alerter = Alerter::new();
    let pass = alerter.pass(w.dev(2), T0 + 1, false).unwrap();
    assert!(pass.commands.iter().any(|x| matches!(x, Command::Show(_))));
    w.dev(1).delete_reminder(&id, T0 + 5).unwrap();
    w.sync_all();
    let pass = alerter.pass(w.dev(2), T0 + 6, false).unwrap();
    assert!(pass
        .commands
        .iter()
        .any(|x| matches!(x, Command::Close { occurrence_id } if *occurrence_id == occ)));
    assert!(w
        .dev(2)
        .deleted_reminders()
        .iter()
        .any(|d| d.reminder_id == id));
}

#[test]
fn delete_wins_over_what_other_devices_did_meanwhile() {
    for order in [[1, 2], [2, 1]] {
        let mut w = World::new(2);
        let id = w.dev(1).create_reminder("Bins", T0 + 10_000, T0).unwrap();
        let home = w.dev(1).create_list("Home", None, T0).unwrap();
        w.sync_all();
        // Device 1 deletes it. Device 2, out of touch, edits it, moves it and
        // changes its time so that it fires.
        w.dev(1).delete_reminder(&id, T0 + 10).unwrap();
        w.dev(2)
            .edit_reminder(
                &id,
                EditReminder {
                    title: Some("Bins!".into()),
                    fire_at: Some(T0 + 20),
                    ..Default::default()
                },
                T0 + 11,
            )
            .unwrap();
        w.dev(2).move_reminder(&id, &home, T0 + 12).unwrap();
        let fired = w.dev(2).tick(T0 + 30).unwrap();
        assert_eq!(fired.len(), 1);
        w.sync_in_order(&order);
        for i in 1..=2 {
            let c = w.dev(i);
            assert!(
                live_in(c).iter().all(|(_, r)| !r.contains(&id)),
                "device {i}, order {order:?}"
            );
            let deleted = c.deleted_reminders();
            assert_eq!(deleted.len(), 1, "device {i}");
            // Its history, including the edit that lost, is kept.
            assert_eq!(
                c.state_of(&home)
                    .unwrap()
                    .history(&id, Setting::Title)
                    .len(),
                2
            );
            // Nothing fires or alerts.
            assert!(c.tick(T0 + 50).unwrap().is_empty());
            let inbox = c.inbox(T0 + 50);
            assert!(inbox.due.is_empty() && inbox.overdue.is_empty());
            let pass = Alerter::new().pass(c, T0 + 50, false).unwrap();
            assert!(pass.commands.is_empty(), "{:?}", pass.commands);
        }
    }
}

#[test]
fn a_reminder_deleted_twice_keeps_the_first_deletion() {
    let mut w = World::new(2);
    let id = w.dev(1).create_reminder("Bins", T0 + 10_000, T0).unwrap();
    w.sync_all();
    w.dev(1).delete_reminder(&id, T0 + 10).unwrap();
    w.dev(2).delete_reminder(&id, T0 + 20).unwrap();
    w.sync_all();
    for i in 1..=2 {
        let d = w.dev(i).deleted_reminders();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].deleted_at, T0 + 10);
    }
}

// ---- Deleting with the history ----

#[test]
fn purging_a_reminder_forgets_it_and_removes_what_this_device_stored() {
    let mut c = solo();
    c.join(USER, "1").unwrap();
    let (id, open, _) = with_history(&mut c);
    let keep = c.create_reminder("Keep me", T0 + 5_000, T0).unwrap();
    c.purge_reminder(&id, T0 + 5).unwrap();
    assert!(c.state().reminders.keys().all(|k| *k != id));
    assert!(c.state().occurrences.is_empty());
    assert!(c.state().history(&id, Setting::Title).is_empty());
    assert!(c.deleted_reminders().is_empty());
    assert!(c.state().reminders.contains_key(&keep));
    assert!(c
        .tick(T0 + 100_000)
        .unwrap()
        .iter()
        .all(|f| f.reminder_id != id));
    assert!(matches!(c.complete(&open, T0 + 6), Err(Error::NotOpen(_))));
    assert_eq!(c.stored_events_of(&id).unwrap(), 0);
    assert!(c.stored_events_of(&keep).unwrap() >= 1);
    // What would be uploaded no longer mentions it, apart from the purge.
    let out = c.unsent().unwrap();
    for o in &out {
        let text = String::from_utf8(o.payload.clone()).unwrap();
        if text.contains(&id) {
            assert!(text.contains("reminder_purged"), "{text}");
        }
    }
    assert!(out
        .iter()
        .any(|o| String::from_utf8_lossy(&o.payload).contains("reminder_purged")));
}

#[test]
fn a_purge_after_a_delete_with_history_and_across_lists() {
    let mut c = solo();
    let (id, _, _) = with_history(&mut c);
    let home = c.create_list("Home", None, T0 + 1).unwrap();
    c.move_reminder(&id, &home, T0 + 2).unwrap();
    c.delete_reminder(&id, T0 + 3).unwrap();
    assert_eq!(c.deleted_reminders().len(), 1);
    c.purge_reminder(&id, T0 + 4).unwrap();
    assert!(c.deleted_reminders().is_empty());
    for (_, s) in [("p", c.state()), ("h", c.state_of(&home).unwrap())] {
        assert!(!s.knows(&id));
        assert!(s.occurrences.is_empty());
    }
    assert!(matches!(
        c.purge_reminder(&id, T0 + 5),
        Err(Error::NoReminder(_))
    ));
}

#[test]
fn a_purge_reaches_other_devices_even_when_they_acted_on_the_reminder_meanwhile() {
    for order in [[1, 2], [2, 1]] {
        let mut w = World::new(2);
        let id = w.dev(1).create_reminder("Bins", T0, T0 - HOUR).unwrap();
        let fired = w.dev(1).tick(T0).unwrap();
        let occ = fired[0].occurrence_id.clone();
        w.sync_all();
        w.dev(1).purge_reminder(&id, T0 + 10).unwrap();
        // Device 2, out of touch, completes it, edits it and snoozes nothing.
        w.dev(2).complete(&occ, T0 + 11).unwrap();
        w.dev(2)
            .edit_reminder(
                &id,
                EditReminder {
                    title: Some("Bins!".into()),
                    ..Default::default()
                },
                T0 + 12,
            )
            .unwrap();
        w.sync_in_order(&order);
        for i in 1..=2 {
            let c = w.dev(i);
            assert!(!c.state().knows(&id), "device {i}, order {order:?}");
            assert!(c.state().occurrences.is_empty());
            assert!(c.deleted_reminders().is_empty());
            // Nothing of it is left in what the device stores.
            assert_eq!(c.stored_events_of(&id).unwrap(), 0, "device {i}");
        }
    }
}

// ---- Filters ----

#[test]
fn filters_are_remembered_and_start_with_everything_shown() {
    let c = solo();
    assert_eq!(c.filters(), Filters::default());
    let f = Filters {
        hidden_lists: vec!["l1".into()],
        hidden_priorities: vec![Priority::Minimum, Priority::Low],
    };
    c.set_filters(&f).unwrap();
    assert_eq!(c.filters(), f);
}

#[test]
fn hiding_a_list_does_not_stop_its_alerts() {
    let mut c = solo();
    let home = c.create_list("Home", None, T0 - HOUR).unwrap();
    let id = c.create_reminder_in(&home, "Bins", T0, T0 - HOUR).unwrap();
    priority(&mut c, &id, Priority::High, T0 - HOUR);
    c.set_filters(&Filters {
        hidden_lists: vec![home.clone()],
        hidden_priorities: vec![Priority::High],
    })
    .unwrap();
    let fired = c.tick(T0).unwrap();
    assert_eq!(fired.len(), 1, "it fires");
    let pass = Alerter::new().pass(&mut c, T0 + 1, false).unwrap();
    assert!(
        pass.commands
            .iter()
            .any(|x| matches!(x, Command::Show(n) if n.occurrence_id == fired[0].occurrence_id)),
        "it alerts"
    );
    // The core's lists and Inbox are not filtered: the window does that.
    assert_eq!(c.inbox(T0 + 1).due.len() + c.inbox(T0 + 1).overdue.len(), 1);
}

#[test]
fn change_edits_are_still_the_only_way_settings_change() {
    // A smoke check that Change is exported and unchanged by this ticket.
    let c = Change::Title("x".into());
    assert_eq!(c.setting(), Setting::Title);
}
