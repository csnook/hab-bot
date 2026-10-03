//! Schedules, expected occurrences and missed occurrences, through the core's
//! own API. Time is passed in, so every test says what the clock reads.

use hab_core::{
    ClosingKind, Core, EditReminder, Error, Event, Fired, Pattern, Schedule, State, StoredEvent,
};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

const USER: &str = "u1";

/// Unix seconds of a local time in a zone.
fn at(zone: &str, local: &str) -> i64 {
    let dt: DateTime = local.parse().unwrap();
    TimeZone::get(zone)
        .unwrap()
        .to_zoned(dt)
        .unwrap()
        .timestamp()
        .as_second()
}

fn schedule(start: &str, rule: &str) -> Schedule {
    Schedule {
        start: start.into(),
        rule: rule.into(),
    }
}

fn core_in(zone: &str) -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone(zone).unwrap();
    c
}

fn open_count(c: &Core, rid: &str) -> usize {
    c.state()
        .occurrences
        .values()
        .filter(|o| o.reminder_id == rid && o.is_open())
        .count()
}

fn kinds(c: &Core, rid: &str) -> Vec<(i64, Option<ClosingKind>)> {
    let mut v: Vec<_> = c
        .state()
        .occurrences
        .values()
        .filter(|o| o.reminder_id == rid)
        .map(|o| (o.scheduled_at, o.closing.as_ref().map(|c| c.kind)))
        .collect();
    v.sort();
    v
}

#[test]
fn a_daily_schedule_fires_each_day_and_only_once_per_instance() {
    let ny = "America/New_York";
    let mut c = core_in(ny);
    let made = at(ny, "2026-10-01T06:00:00");
    let rid = c
        .create_recurring_reminder(
            "Medicine",
            vec![schedule("2026-10-01T07:00:00", "FREQ=DAILY")],
            None,
            made,
        )
        .unwrap();
    assert!(c.tick(at(ny, "2026-10-01T06:59:59")).unwrap().is_empty());
    let fired = c.tick(at(ny, "2026-10-01T07:00:00")).unwrap();
    assert_eq!(fired.len(), 1);
    let first = at(ny, "2026-10-01T07:00:00");
    assert_eq!(fired[0].occurrence_id, format!("{rid}@{first}"));
    // Ticking again fires nothing new.
    assert!(c.tick(at(ny, "2026-10-01T07:00:30")).unwrap().is_empty());
    assert_eq!(c.next_fire_at(), Some(at(ny, "2026-10-02T07:00:00")));
}

#[test]
fn a_new_schedule_instance_marks_the_open_occurrence_missed() {
    let mut c = core_in("UTC");
    let rid = c
        .create_recurring_reminder(
            "Medicine",
            vec![schedule("2026-10-01T08:00:00", "FREQ=DAILY;BYHOUR=8,20")],
            Some("UTC"),
            at("UTC", "2026-10-01T00:00:00"),
        )
        .unwrap();
    c.tick(at("UTC", "2026-10-01T08:00:00")).unwrap();
    assert_eq!(open_count(&c, &rid), 1);
    // The 8:00 one is left open; the 20:00 one fires and misses it.
    c.tick(at("UTC", "2026-10-01T20:00:00")).unwrap();
    assert_eq!(open_count(&c, &rid), 1);
    let k = kinds(&c, &rid);
    assert_eq!(
        k,
        vec![
            (at("UTC", "2026-10-01T08:00:00"), Some(ClosingKind::Missed)),
            (at("UTC", "2026-10-01T20:00:00"), None),
        ]
    );
    let missed = c.state().occurrences[&format!("{rid}@{}", at("UTC", "2026-10-01T08:00:00"))]
        .closing
        .clone()
        .unwrap();
    assert_eq!(missed.at, at("UTC", "2026-10-01T20:00:00"));
    // A completed one is not marked missed by the next.
    let occ = format!("{rid}@{}", at("UTC", "2026-10-01T20:00:00"));
    c.complete(&occ, at("UTC", "2026-10-01T20:05:00")).unwrap();
    c.tick(at("UTC", "2026-10-02T08:00:00")).unwrap();
    assert_eq!(kinds(&c, &rid)[1].1, Some(ClosingKind::Completed));
    assert_eq!(open_count(&c, &rid), 1);
}

#[test]
fn several_schedules_on_one_reminder_fire_in_turn() {
    let mut c = core_in("UTC");
    let rid = c
        .create_recurring_reminder(
            "Pills",
            vec![
                schedule("2026-10-01T08:00:00", "FREQ=DAILY"),
                schedule("2026-10-01T20:00:00", "FREQ=DAILY"),
                // The same time as the first: still one occurrence.
                schedule("2026-10-01T08:00:00", "FREQ=WEEKLY"),
            ],
            None,
            at("UTC", "2026-10-01T00:00:00"),
        )
        .unwrap();
    let fired = c.tick(at("UTC", "2026-10-01T08:00:00")).unwrap();
    assert_eq!(fired.len(), 1);
    c.tick(at("UTC", "2026-10-01T20:00:00")).unwrap();
    assert_eq!(kinds(&c, &rid).len(), 2);
    assert_eq!(open_count(&c, &rid), 1);
    assert_eq!(
        c.expected(
            at("UTC", "2026-10-01T21:00:00"),
            at("UTC", "2026-10-02T23:00:00")
        )
        .len(),
        2
    );
}

#[test]
fn a_floating_reminder_follows_the_device_and_a_pinned_one_does_not() {
    let made = at("America/New_York", "2026-10-01T00:00:00");
    let daily = || vec![schedule("2026-10-01T07:00:00", "FREQ=DAILY")];
    let mut c = core_in("America/New_York");
    let floating = c
        .create_recurring_reminder("Floating", daily(), None, made)
        .unwrap();
    let pinned = c
        .create_recurring_reminder("Pinned", daily(), Some("America/New_York"), made)
        .unwrap();
    // Day one, at home.
    c.tick(at("America/New_York", "2026-10-01T07:00:00"))
        .unwrap();
    // The device flies to Tokyo overnight.
    c.set_device_zone("Asia/Tokyo").unwrap();
    assert_eq!(c.device_zone(), "Asia/Tokyo");
    let next_float = at("Asia/Tokyo", "2026-10-03T07:00:00");
    let next_pinned = at("America/New_York", "2026-10-02T07:00:00");
    // 2026-10-02 07:00 in Tokyo is already past when this tick runs, so
    // floating fires 7:00 Tokyo wherever the device now is.
    let now = at("Asia/Tokyo", "2026-10-03T07:00:00");
    let fired: Vec<Fired> = c.tick(now).unwrap();
    let ids: Vec<&str> = fired.iter().map(|f| f.reminder_id.as_str()).collect();
    assert!(ids.contains(&floating.as_str()) && ids.contains(&pinned.as_str()));
    let last = |rid: &str| kinds(&c, rid).last().unwrap().0;
    assert_eq!(last(&floating), next_float);
    // The pinned reminder stays on New York's 7:00: Oct 2 and Oct 3 passed
    // (Oct 3 07:00 Tokyo is Oct 2 18:00 in New York, so only Oct 2 had come).
    assert_eq!(last(&pinned), next_pinned);
    // And a later move back changes the next floating time again.
    c.set_device_zone("America/New_York").unwrap();
    assert_eq!(
        c.next_fire_at().unwrap(),
        at("America/New_York", "2026-10-03T07:00:00")
            .min(at("America/New_York", "2026-10-03T07:00:00"))
    );
}

#[test]
fn the_spring_forward_night_fires_2_30_at_3_00() {
    let ny = "America/New_York";
    let mut c = core_in(ny);
    let rid = c
        .create_recurring_reminder(
            "Night",
            vec![schedule("2026-03-07T02:30:00", "FREQ=DAILY")],
            None,
            at(ny, "2026-03-07T12:00:00"),
        )
        .unwrap();
    assert!(c.tick(at(ny, "2026-03-08T01:59:59")).unwrap().is_empty());
    let fired = c.tick(at(ny, "2026-03-08T03:00:00")).unwrap();
    assert_eq!(fired.len(), 1);
    let o = c.state().occurrences.values().next().unwrap();
    assert_eq!(o.scheduled_at, at(ny, "2026-03-08T03:00:00"));
    assert_eq!(o.scheduled_at, 1_772_953_200); // 07:00:00 UTC
                                               // The next night is an ordinary 2:30.
    assert_eq!(c.next_fire_at(), Some(at(ny, "2026-03-09T02:30:00")));
    assert_eq!(open_count(&c, &rid), 1);
}

#[test]
fn the_fall_back_night_fires_1_30_once_the_first_time() {
    let ny = "America/New_York";
    let mut c = core_in(ny);
    let rid = c
        .create_recurring_reminder(
            "Night",
            vec![schedule("2026-10-31T01:30:00", "FREQ=DAILY")],
            None,
            at(ny, "2026-10-31T12:00:00"),
        )
        .unwrap();
    let first = 1_793_511_000; // 2026-11-01T05:30:00Z, on daylight time
    assert!(c.tick(first - 1).unwrap().is_empty());
    assert_eq!(c.tick(first).unwrap().len(), 1);
    // The second 1:30, an hour later, is the same instance: nothing fires.
    assert!(c.tick(first + 3600).unwrap().is_empty());
    assert!(c.tick(first + 3600 + 1).unwrap().is_empty());
    assert_eq!(kinds(&c, &rid).len(), 1);
    assert_eq!(c.next_fire_at(), Some(at(ny, "2026-11-02T01:30:00")));
}

#[test]
fn a_device_that_slept_fires_late_and_records_the_instances_it_passed_as_missed() {
    let ny = "America/New_York";
    let mut c = core_in(ny);
    let rid = c
        .create_recurring_reminder(
            "Bins",
            vec![schedule("2026-10-01T07:00:00", "FREQ=DAILY")],
            None,
            at(ny, "2026-10-01T00:00:00"),
        )
        .unwrap();
    c.tick(at(ny, "2026-10-01T07:00:00")).unwrap();
    // Asleep until Oct 4 at 9:15: Oct 2, 3 and 4 have all come.
    let wake = at(ny, "2026-10-04T09:15:00");
    let fired = c.tick(wake).unwrap();
    assert_eq!(fired.len(), 1, "only the latest alerts");
    let k = kinds(&c, &rid);
    assert_eq!(k.len(), 4);
    assert_eq!(k[0].1, Some(ClosingKind::Missed));
    assert_eq!(k[1].1, Some(ClosingKind::Missed));
    assert_eq!(k[2].1, Some(ClosingKind::Missed));
    assert_eq!(k[3], (at(ny, "2026-10-04T07:00:00"), None));
    assert_eq!(open_count(&c, &rid), 1);
    // It fires late, but its times count from the scheduled time.
    let due = c.snapshot().due;
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].scheduled_at, at(ny, "2026-10-04T07:00:00"));
    assert_eq!(due[0].fired_at, wake);
    // The missed ones were missed when the next one was due.
    let second = &c.state().occurrences[&format!("{rid}@{}", at(ny, "2026-10-02T07:00:00"))];
    assert_eq!(
        second.closing.as_ref().unwrap().at,
        at(ny, "2026-10-03T07:00:00")
    );
}

#[test]
fn nothing_fires_for_instants_before_the_reminder_was_made() {
    let mut c = core_in("UTC");
    let rid = c
        .create_recurring_reminder(
            "Late setup",
            vec![schedule("2026-10-01T07:00:00", "FREQ=DAILY")],
            None,
            at("UTC", "2026-10-03T08:00:00"),
        )
        .unwrap();
    assert!(c.tick(at("UTC", "2026-10-03T08:30:00")).unwrap().is_empty());
    assert!(kinds(&c, &rid).is_empty());
    assert_eq!(c.tick(at("UTC", "2026-10-04T07:00:00")).unwrap().len(), 1);
}

#[test]
fn expected_occurrences_and_the_inbox_sections() {
    let mut c = core_in("UTC");
    let rid = c
        .create_recurring_reminder(
            "Pills",
            vec![schedule(
                "2026-10-01T06:00:00",
                "FREQ=DAILY;BYHOUR=6,12,18,23",
            )],
            None,
            at("UTC", "2026-10-03T00:00:00"),
        )
        .unwrap();
    let once = c
        .create_reminder(
            "Call",
            at("UTC", "2026-10-03T20:00:00"),
            at("UTC", "2026-10-01T00:00:00"),
        )
        .unwrap();
    // The 6:00 one fires and is missed by 12:00, which fires.
    c.tick(at("UTC", "2026-10-03T06:00:00")).unwrap();
    let now = at("UTC", "2026-10-03T12:30:00");
    c.tick(now).unwrap();

    let inbox = c.inbox(now);
    let later: Vec<(String, i64)> = inbox
        .later_today
        .iter()
        .map(|e| (e.title.clone(), e.scheduled_at))
        .collect();
    assert_eq!(
        later,
        vec![
            ("Pills".to_string(), at("UTC", "2026-10-03T18:00:00")),
            ("Call".to_string(), at("UTC", "2026-10-03T20:00:00")),
            ("Pills".to_string(), at("UTC", "2026-10-03T23:00:00")),
        ]
    );
    let _ = once;
    // The 6:00 occurrence, missed at 12:00 today, is under Earlier today.
    assert_eq!(inbox.earlier_today.len(), 1);
    assert_eq!(inbox.earlier_today[0].kind, ClosingKind::Missed);
    assert_eq!(
        inbox.earlier_today[0].scheduled_at,
        at("UTC", "2026-10-03T06:00:00")
    );
    // Closing the 12:00 one puts it there too, latest first.
    let occ = format!("{rid}@{}", at("UTC", "2026-10-03T12:00:00"));
    c.complete(&occ, now + 60).unwrap();
    let earlier = c.inbox(now + 60).earlier_today;
    assert_eq!(earlier.len(), 2);
    assert_eq!(earlier[0].kind, ClosingKind::Completed);
    // Tomorrow's expected occurrences are predicted for any range asked.
    let range = c.expected(now, at("UTC", "2026-10-05T00:00:00"));
    assert_eq!(range.len(), 2 + 1 + 4);
    // The next day's inbox starts empty.
    assert!(c
        .inbox(at("UTC", "2026-10-04T00:00:01"))
        .earlier_today
        .is_empty());
}

#[test]
fn the_days_of_the_inbox_follow_the_device_time_zone() {
    let mut c = core_in("Asia/Tokyo");
    c.create_reminder("Call", at("Asia/Tokyo", "2026-10-03T23:00:00"), 0)
        .unwrap();
    let now = at("Asia/Tokyo", "2026-10-03T09:00:00");
    assert_eq!(c.inbox(now).later_today.len(), 1);
    c.set_device_zone("America/Los_Angeles").unwrap();
    // 09:00 Tokyo is the evening before in Los Angeles, and 23:00 Tokyo is
    // still that evening's tomorrow morning.
    assert_eq!(c.inbox(now).later_today.len(), 0);
}

#[test]
fn editing_schedules_and_the_zone() {
    let mut c = core_in("UTC");
    let rid = c
        .create_recurring_reminder(
            "Bins",
            vec![schedule("2026-10-01T07:00:00", "FREQ=DAILY")],
            None,
            at("UTC", "2026-10-01T00:00:00"),
        )
        .unwrap();
    c.tick(at("UTC", "2026-10-01T07:00:00")).unwrap();
    // Weekly on Thursdays instead, pinned to Berlin from now on.
    let thursdays = Schedule::from_pattern(
        &Pattern::Weekly {
            days: vec!["TH".into()],
        },
        "2026-10-01",
        "07:00",
    )
    .unwrap();
    c.edit_reminder(
        &rid,
        EditReminder {
            schedules: Some(vec![thursdays]),
            zone: Some(Some("Europe/Berlin".into())),
            ..Default::default()
        },
        at("UTC", "2026-10-01T08:00:00"),
    )
    .unwrap();
    let r = &c.state().reminders[&rid];
    assert_eq!(r.zone.as_deref(), Some("Europe/Berlin"));
    assert_eq!(
        c.next_fire_at(),
        Some(at("Europe/Berlin", "2026-10-08T07:00:00"))
    );
    // Bad edits are refused, and change nothing.
    let bad = c.edit_reminder(
        &rid,
        EditReminder {
            zone: Some(Some("Mars/Olympus".into())),
            ..Default::default()
        },
        at("UTC", "2026-10-01T09:00:00"),
    );
    assert!(matches!(bad, Err(Error::BadZone(_))));
    let bad = c.create_recurring_reminder(
        "X",
        vec![schedule("2026-10-01T07:00:00", "FREQ=HOURLY")],
        None,
        0,
    );
    assert!(matches!(bad, Err(Error::BadSchedule(_))));
    assert!(matches!(
        c.set_device_zone("Nowhere/Land"),
        Err(Error::BadZone(_))
    ));
    // Back to floating.
    c.edit_reminder(
        &rid,
        EditReminder {
            zone: Some(None),
            ..Default::default()
        },
        at("UTC", "2026-10-01T10:00:00"),
    )
    .unwrap();
    assert_eq!(c.state().reminders[&rid].zone, None);
}

/// Devices of one user, each with its own clock and zone, and the server's
/// numbering.
struct World {
    devices: Vec<Core>,
    log: Vec<(String, String, u32, Vec<u8>)>,
    list: String,
}

impl World {
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
    fn sync_all(&mut self) {
        for i in 0..self.devices.len() {
            let id = (i + 1).to_string();
            for o in self.devices[i].unsent().unwrap() {
                self.log
                    .push((o.event_id.clone(), id.clone(), o.format, o.payload.clone()));
                let seq = self.log.len() as i64;
                self.devices[i].mark_sent(&o.event_id, seq).unwrap();
            }
        }
        for i in 0..self.devices.len() {
            for (n, (event_id, device, format, payload)) in self.log.clone().iter().enumerate() {
                self.devices[i]
                    .receive(
                        &self.list.clone(),
                        n as i64 + 1,
                        event_id,
                        device,
                        *format,
                        payload,
                    )
                    .unwrap();
            }
        }
    }
}

#[test]
fn two_devices_firing_the_same_instance_make_one_occurrence() {
    let mut w = World::new(2);
    let t = at("UTC", "2026-10-01T00:00:00");
    for d in &w.devices {
        d.set_device_zone("UTC").unwrap();
    }
    let rid = w.devices[0]
        .create_recurring_reminder(
            "Bins",
            vec![schedule("2026-10-01T07:00:00", "FREQ=DAILY")],
            None,
            t,
        )
        .unwrap();
    w.sync_all();
    let now = at("UTC", "2026-10-01T07:00:00");
    // Both fire it before either hears from the other.
    assert_eq!(w.devices[0].tick(now).unwrap().len(), 1);
    assert_eq!(w.devices[1].tick(now + 3).unwrap().len(), 1);
    w.sync_all();
    for d in &w.devices {
        assert_eq!(kinds(d, &rid).len(), 1);
        assert_eq!(open_count(d, &rid), 1);
        // And ticking again, now that they know, fires nothing.
    }
    assert!(w.devices[0].tick(now + 10).unwrap().is_empty());
    assert!(w.devices[1].tick(now + 10).unwrap().is_empty());
    w.sync_all();
    assert_eq!(kinds(&w.devices[0], &rid).len(), 1);
}

#[test]
fn a_device_waking_late_finds_another_already_closed_the_occurrence() {
    let mut w = World::new(2);
    let t = at("UTC", "2026-10-01T00:00:00");
    for d in &w.devices {
        d.set_device_zone("UTC").unwrap();
    }
    let rid = w.devices[0]
        .create_recurring_reminder(
            "Bins",
            vec![schedule("2026-10-01T07:00:00", "FREQ=DAILY")],
            None,
            t,
        )
        .unwrap();
    w.sync_all();
    let now = at("UTC", "2026-10-01T07:00:00");
    w.devices[0].tick(now).unwrap();
    let occ = format!("{rid}@{now}");
    w.devices[0].complete(&occ, now + 60).unwrap();
    w.sync_all();
    // Device 2 wakes at 9:00, after syncing: the instance is already
    // closed, so nothing fires.
    assert!(w.devices[1].tick(now + 7200).unwrap().is_empty());
    assert_eq!(
        w.devices[1].state().occurrences[&occ]
            .closing
            .as_ref()
            .unwrap()
            .kind,
        ClosingKind::Completed
    );
    // Had it woken without syncing, it fires the instance late, and the
    // completion wins when they meet.
    let mut w = World::new(2);
    for d in &w.devices {
        d.set_device_zone("UTC").unwrap();
    }
    let rid = w.devices[0]
        .create_recurring_reminder(
            "Bins",
            vec![schedule("2026-10-01T07:00:00", "FREQ=DAILY")],
            None,
            t,
        )
        .unwrap();
    w.sync_all();
    w.devices[0].tick(now).unwrap();
    let occ = format!("{rid}@{now}");
    w.devices[0].complete(&occ, now + 60).unwrap();
    assert_eq!(w.devices[1].tick(now + 7200).unwrap().len(), 1);
    w.sync_all();
    for d in &w.devices {
        assert_eq!(kinds(d, &rid), vec![(now, Some(ClosingKind::Completed))]);
    }
}

fn stored(list: &str, n: usize, recorded_at: i64, event: Event) -> StoredEvent {
    StoredEvent {
        list_id: list.into(),
        seq: Some(n as i64),
        event_id: format!("e{n}"),
        device_id: "1".into(),
        author: USER.into(),
        recorded_at,
        event,
    }
}

#[test]
fn openings_in_any_order_leave_one_open_occurrence() {
    let rid = "r1".to_string();
    let created = stored(
        "l",
        1,
        0,
        Event::RecurringReminderCreated {
            reminder_id: rid.clone(),
            title: "Bins".into(),
            schedules: vec![schedule("2026-10-01T07:00:00", "FREQ=DAILY")],
            zone: None,
        },
    );
    let open = |n: usize, at: i64| {
        stored(
            "l",
            n,
            at,
            Event::OccurrenceOpened {
                occurrence_id: format!("{rid}@{at}"),
                reminder_id: rid.clone(),
                scheduled_at: at,
                fired_at: at,
            },
        )
    };
    let events = [open(2, 1000), open(3, 2000), open(4, 3000)];
    let orders: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for order in orders {
        let mut s = State::default();
        s.apply(&created);
        for i in order {
            s.apply(&events[i]);
            s.apply(&events[i]); // duplicates change nothing
        }
        let open_ids: Vec<_> = s
            .occurrences
            .values()
            .filter(|o| o.is_open())
            .map(|o| o.scheduled_at)
            .collect();
        assert_eq!(open_ids, vec![3000], "order {order:?}");
        assert_eq!(s.occurrences.len(), 3);
        // The ones that lost were missed, by nobody.
        let missed = s
            .occurrences
            .values()
            .filter(|o| {
                o.closing
                    .as_ref()
                    .is_some_and(|c| c.kind == ClosingKind::Missed)
            })
            .count();
        assert_eq!(missed, 2);
        assert!(s.reconciliations.is_empty());
    }
    // A completion that arrives for an expired one still beats the miss.
    let mut s = State::default();
    s.apply(&created);
    for e in &events {
        s.apply(e);
    }
    s.apply(&stored(
        "l",
        9,
        5000,
        Event::OccurrenceCompleted {
            occurrence_id: format!("{rid}@1000"),
            completed_at: 1500,
        },
    ));
    assert_eq!(
        s.occurrences[&format!("{rid}@1000")]
            .closing
            .as_ref()
            .unwrap()
            .kind,
        ClosingKind::Completed
    );
    assert!(s.reconciliations.iter().all(|r| r.by != USER));
}

#[test]
fn recurring_events_need_a_reader_of_the_new_format() {
    let e = Event::RecurringReminderCreated {
        reminder_id: "r".into(),
        title: "t".into(),
        schedules: vec![],
        zone: None,
    };
    assert_eq!(e.format(), 2);
    assert_eq!(
        Event::OccurrenceMissed {
            occurrence_id: "o".into(),
            missed_at: 1
        }
        .format(),
        1
    );
    // What a standalone device stores for it is in format 2 too.
    let mut c = core_in("UTC");
    c.join(USER, "1").unwrap();
    c.create_recurring_reminder(
        "x",
        vec![schedule("2026-10-01T07:00:00", "FREQ=DAILY")],
        None,
        0,
    )
    .unwrap();
    c.create_reminder("y", 5, 0).unwrap();
    let formats: Vec<u32> = c.unsent().unwrap().iter().map(|o| o.format).collect();
    assert_eq!(formats, vec![2, 1]);
}
