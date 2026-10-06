//! Countdowns, through the core's own API. Time is passed in, so every test
//! says what the clock reads.

use hab_core::{ClosingKind, Core, Countdown, CountdownUnit, EditReminder, Error, Event, Setting};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

const NY: &str = "America/New_York";
const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

fn at(zone: &str, local: &str) -> i64 {
    let dt: DateTime = local.parse().unwrap();
    TimeZone::get(zone)
        .unwrap()
        .to_zoned(dt)
        .unwrap()
        .timestamp()
        .as_second()
}

fn core_in(zone: &str) -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone(zone).unwrap();
    c
}

fn countdown(amount: u32, unit: CountdownUnit, at: Option<&str>) -> Countdown {
    Countdown {
        amount,
        unit,
        at: at.map(str::to_string),
    }
}

fn open_id(c: &Core, rid: &str) -> String {
    c.state()
        .occurrences
        .values()
        .find(|o| o.reminder_id == rid && o.is_open())
        .expect("an open occurrence")
        .id
        .clone()
}

fn occurrences(c: &Core, rid: &str) -> usize {
    c.state()
        .occurrences
        .values()
        .filter(|o| o.reminder_id == rid)
        .count()
}

#[test]
fn a_countdown_of_hours_counts_elapsed_time() {
    let mut c = core_in(NY);
    let t0 = at(NY, "2026-10-01T08:00:00");
    let rid = c
        .create_countdown_reminder(
            "Dose",
            countdown(8, CountdownUnit::Hours, None),
            None,
            Some(t0),
            t0,
        )
        .unwrap();
    assert!(c.tick(t0 + 8 * HOUR - 1).unwrap().is_empty());
    assert_eq!(c.next_fire_at(), Some(t0 + 8 * HOUR));
    let fired = c.tick(t0 + 8 * HOUR).unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].reminder_id, rid);
    assert_eq!(fired[0].occurrence_id, format!("{rid}@{}", t0 + 8 * HOUR));
    // Ticking again fires nothing new, and nothing is expected while open.
    assert!(c.tick(t0 + 8 * HOUR + 5).unwrap().is_empty());
    assert!(c.expected(t0 + 8 * HOUR, t0 + 30 * DAY).is_empty());
}

#[test]
fn days_can_fire_at_a_time_of_day_in_the_reminders_zone() {
    let mut c = core_in(NY);
    let done = at(NY, "2026-10-01T14:20:00");
    let rid = c
        .create_countdown_reminder(
            "Water the plants",
            countdown(3, CountdownUnit::Days, Some("09:00")),
            None,
            Some(done),
            done,
        )
        .unwrap();
    let due = at(NY, "2026-10-04T09:00:00");
    assert!(c.tick(due - 1).unwrap().is_empty());
    assert_eq!(c.tick(due).unwrap().len(), 1);
    assert_eq!(open_id(&c, &rid), format!("{rid}@{due}"));
}

#[test]
fn a_floating_time_of_day_follows_the_device_and_a_pinned_one_does_not() {
    let done_utc = at("UTC", "2026-10-01T12:00:00");
    for (pinned, device, expect) in [
        // Floating: 9:00 wherever the device is.
        (None, "Asia/Tokyo", at("Asia/Tokyo", "2026-10-04T09:00:00")),
        (None, NY, at(NY, "2026-10-04T09:00:00")),
        // Pinned to New York: 9:00 in New York wherever the device is.
        (Some(NY), "Asia/Tokyo", at(NY, "2026-10-04T09:00:00")),
    ] {
        let mut c = core_in(device);
        c.create_countdown_reminder(
            "Plants",
            countdown(3, CountdownUnit::Days, Some("09:00")),
            pinned,
            Some(done_utc),
            done_utc,
        )
        .unwrap();
        assert_eq!(c.next_fire_at(), Some(expect), "{pinned:?} on {device}");
    }
}

#[test]
fn elapsed_countdowns_ignore_the_time_zone() {
    let t0 = at("UTC", "2026-03-08T04:00:00");
    for (pinned, device) in [
        (None, NY),
        (None, "Asia/Tokyo"),
        (Some("Europe/London"), NY),
    ] {
        let mut c = core_in(device);
        c.create_countdown_reminder(
            "Dose",
            countdown(6, CountdownUnit::Hours, None),
            pinned,
            Some(t0),
            t0,
        )
        .unwrap();
        // Six hours later, across New York's spring-forward gap or not.
        assert_eq!(c.next_fire_at(), Some(t0 + 6 * HOUR));
    }
}

#[test]
fn creating_with_never_fires_at_once_and_with_a_last_time_counts_from_it() {
    let mut c = core_in(NY);
    let now = at(NY, "2026-10-01T10:00:00");
    let never = c
        .create_countdown_reminder(
            "Never done",
            countdown(2, CountdownUnit::Days, None),
            None,
            None,
            now,
        )
        .unwrap();
    // Done a day ago: due in a day.
    let ago = c
        .create_countdown_reminder(
            "Done yesterday",
            countdown(2, CountdownUnit::Days, None),
            None,
            Some(now - DAY),
            now,
        )
        .unwrap();
    // Done three days ago: it is already due.
    let late = c
        .create_countdown_reminder(
            "Done long ago",
            countdown(2, CountdownUnit::Days, None),
            None,
            Some(now - 3 * DAY),
            now,
        )
        .unwrap();
    let fired = c.tick(now).unwrap();
    let ids: Vec<_> = fired.iter().map(|f| f.reminder_id.clone()).collect();
    assert!(ids.contains(&never) && ids.contains(&late) && !ids.contains(&ago));
    // The one done long ago was due a day ago, and counts overdue from then.
    let o = &c.state().occurrences[&open_id(&c, &late)];
    assert_eq!(o.scheduled_at, now - DAY);
    assert_eq!(o.fired_at, now);
    let n = &c.state().occurrences[&open_id(&c, &never)];
    assert_eq!(n.scheduled_at, now);
    // A time that hasn't come can't be when it was last done.
    assert!(matches!(
        c.create_countdown_reminder(
            "x",
            countdown(1, CountdownUnit::Days, None),
            None,
            Some(now + 1),
            now
        ),
        Err(Error::InTheFuture)
    ));
}

#[test]
fn a_completion_restarts_from_its_recorded_time() {
    let mut c = core_in(NY);
    let t0 = at(NY, "2026-10-01T08:00:00");
    let rid = c
        .create_countdown_reminder(
            "Plants",
            countdown(3, CountdownUnit::Days, Some("09:00")),
            None,
            None,
            t0,
        )
        .unwrap();
    c.tick(t0).unwrap();
    let occ = open_id(&c, &rid);
    // Done at 9:40 but tapped at 11:00.
    let recorded = at(NY, "2026-10-01T09:40:00");
    let tapped = at(NY, "2026-10-01T11:00:00");
    c.complete_at(&occ, recorded, tapped).unwrap();
    assert_eq!(c.next_fire_at(), Some(at(NY, "2026-10-04T09:00:00")));
    // Elapsed countdowns count from the recorded time too.
    let hrid = c
        .create_countdown_reminder(
            "Dose",
            countdown(8, CountdownUnit::Hours, None),
            None,
            None,
            tapped,
        )
        .unwrap();
    c.tick(tapped).unwrap();
    let hocc = open_id(&c, &hrid);
    c.complete_at(&hocc, recorded, tapped).unwrap();
    assert_eq!(
        c.expected(tapped, tapped + DAY)
            .iter()
            .find(|e| e.reminder_id == hrid)
            .map(|e| e.scheduled_at),
        Some(recorded + 8 * HOUR)
    );
    // A time still to come can't be recorded.
    assert!(matches!(
        c.complete_at("nope", tapped + 1, tapped),
        Err(Error::InTheFuture)
    ));
}

#[test]
fn a_skip_or_a_miss_restarts_from_when_the_occurrence_closed() {
    let mut c = core_in(NY);
    let t0 = at(NY, "2026-10-01T08:00:00");
    let skipped = c
        .create_countdown_reminder(
            "Skipped",
            countdown(4, CountdownUnit::Hours, None),
            None,
            None,
            t0,
        )
        .unwrap();
    let missed = c
        .create_countdown_reminder(
            "Missed",
            countdown(4, CountdownUnit::Hours, None),
            None,
            None,
            t0,
        )
        .unwrap();
    c.edit_reminder(
        &missed,
        EditReminder {
            expiry: Some(Some(HOUR)),
            ..Default::default()
        },
        t0,
    )
    .unwrap();
    c.tick(t0).unwrap();
    let skip_at = t0 + 30 * 60;
    c.skip(&open_id(&c, &skipped), Some("away"), skip_at)
        .unwrap();
    // The miss is as of when the expiry came, not when the device noticed.
    c.tick(t0 + 3 * HOUR).unwrap();
    let o = c
        .state()
        .occurrences
        .values()
        .find(|o| o.reminder_id == missed)
        .unwrap();
    assert_eq!(o.closing.as_ref().unwrap().kind, ClosingKind::Missed);
    let expected = |c: &Core, rid: &str| {
        c.expected(t0 + 3 * HOUR, t0 + 3 * DAY)
            .into_iter()
            .find(|e| e.reminder_id == rid)
            .map(|e| e.scheduled_at)
    };
    assert_eq!(expected(&c, &skipped), Some(skip_at + 4 * HOUR));
    assert_eq!(expected(&c, &missed), Some(t0 + HOUR + 4 * HOUR));
}

#[test]
fn completing_before_it_fires_restarts_it_and_cancels_the_pending_firing() {
    let mut c = core_in(NY);
    let t0 = at(NY, "2026-10-01T08:00:00");
    let rid = c
        .create_countdown_reminder(
            "Plants",
            countdown(3, CountdownUnit::Days, None),
            None,
            Some(t0),
            t0,
        )
        .unwrap();
    let pending = t0 + 3 * DAY;
    assert_eq!(c.next_fire_at(), Some(pending));
    // Watered after two days, before it fires.
    let watered = t0 + 2 * DAY;
    let occ = c.complete_expected(&rid, watered, watered).unwrap();
    assert_eq!(occ, format!("{rid}@{pending}"));
    // The pending firing is gone: nothing fires at the old time.
    assert!(c.tick(pending).unwrap().is_empty());
    assert_eq!(occurrences(&c, &rid), 1);
    assert_eq!(c.next_fire_at(), Some(watered + 3 * DAY));
    let o = &c.state().occurrences[&occ];
    assert_eq!(o.completed().unwrap().1, watered);
    // It fires again on the new time.
    assert_eq!(c.tick(watered + 3 * DAY).unwrap().len(), 1);
    // Only a countdown reminder can be completed that way.
    let one_off = c.create_reminder("Once", watered + DAY, watered).unwrap();
    assert!(matches!(
        c.complete_expected(&one_off, watered, watered),
        Err(Error::NotCountdown(_))
    ));
}

#[test]
fn skipping_before_it_fires_restarts_from_now() {
    let mut c = core_in(NY);
    let t0 = at(NY, "2026-10-01T08:00:00");
    let rid = c
        .create_countdown_reminder(
            "Plants",
            countdown(3, CountdownUnit::Days, None),
            None,
            Some(t0),
            t0,
        )
        .unwrap();
    let now = t0 + DAY;
    c.skip_expected(&rid, Some("away"), now).unwrap();
    assert_eq!(c.next_fire_at(), Some(now + 3 * DAY));
    assert!(c.tick(t0 + 3 * DAY).unwrap().is_empty());
}

#[test]
fn a_countdown_predicts_only_its_next_occurrence() {
    let mut c = core_in(NY);
    let t0 = at(NY, "2026-10-01T08:00:00");
    let rid = c
        .create_countdown_reminder(
            "Dose",
            countdown(2, CountdownUnit::Hours, None),
            None,
            Some(t0),
            t0,
        )
        .unwrap();
    // A month is a lot of two-hour countdowns, and still only one is expected.
    let expected = c.expected(t0, t0 + 30 * DAY);
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].scheduled_at, t0 + 2 * HOUR);
    // Later today shows it only if it falls today.
    let inbox = c.inbox(t0);
    assert_eq!(inbox.later_today.len(), 1);
    // After it closes the next one is predicted from the closing.
    c.tick(t0 + 2 * HOUR).unwrap();
    assert!(c.expected(t0 + 2 * HOUR, t0 + 30 * DAY).is_empty());
    let occ = open_id(&c, &rid);
    c.complete(&occ, t0 + 3 * HOUR).unwrap();
    let expected = c.expected(t0 + 3 * HOUR, t0 + 30 * DAY);
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].scheduled_at, t0 + 5 * HOUR);
    // The snapshot lists it with when it fires next.
    let items = c.snapshot().countdowns;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].next_at, Some(t0 + 5 * HOUR));
}

#[test]
fn a_device_that_slept_fires_once_and_counts_overdue_from_the_countdown() {
    let mut c = core_in(NY);
    let t0 = at(NY, "2026-10-01T08:00:00");
    let rid = c
        .create_countdown_reminder(
            "Dose",
            countdown(1, CountdownUnit::Hours, None),
            None,
            Some(t0),
            t0,
        )
        .unwrap();
    let fired = c.tick(t0 + 10 * DAY).unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(occurrences(&c, &rid), 1);
    assert_eq!(
        c.state().occurrences[&open_id(&c, &rid)].scheduled_at,
        t0 + HOUR
    );
}

/// Devices of one user and the server's numbering.
struct World {
    devices: Vec<Core>,
    log: Vec<(String, String, u32, Vec<u8>)>,
    list: String,
}

impl World {
    fn new(n: usize) -> World {
        let mut first = Core::open_in_memory().unwrap();
        first.join("u1", "1").unwrap();
        let list = first.personal_list_id().to_string();
        let mut devices = vec![first];
        for i in 2..=n {
            let mut c = Core::open_in_memory().unwrap();
            c.join("u1", &i.to_string()).unwrap();
            c.use_personal_list(&list).unwrap();
            devices.push(c);
        }
        for d in &devices {
            d.set_device_zone("UTC").unwrap();
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
fn two_devices_firing_the_same_countdown_make_one_occurrence() {
    let mut w = World::new(2);
    let t0 = at("UTC", "2026-10-01T08:00:00");
    let rid = w.devices[0]
        .create_countdown_reminder(
            "Dose",
            countdown(1, CountdownUnit::Hours, None),
            None,
            Some(t0),
            t0,
        )
        .unwrap();
    w.sync_all();
    assert_eq!(w.devices[0].tick(t0 + HOUR).unwrap().len(), 1);
    assert_eq!(w.devices[1].tick(t0 + HOUR + 3).unwrap().len(), 1);
    w.sync_all();
    for d in &w.devices {
        assert_eq!(occurrences(d, &rid), 1);
    }
    assert!(w.devices[0].tick(t0 + HOUR + 10).unwrap().is_empty());
    assert!(w.devices[1].tick(t0 + HOUR + 10).unwrap().is_empty());
}

#[test]
fn completing_ahead_on_one_device_cancels_the_firing_on_another() {
    let mut w = World::new(2);
    let t0 = at("UTC", "2026-10-01T08:00:00");
    let rid = w.devices[0]
        .create_countdown_reminder(
            "Plants",
            countdown(3, CountdownUnit::Days, None),
            None,
            Some(t0),
            t0,
        )
        .unwrap();
    w.sync_all();
    let watered = t0 + DAY;
    w.devices[0]
        .complete_expected(&rid, watered, watered)
        .unwrap();
    w.sync_all();
    // The other device hears of it before the old time comes.
    assert!(w.devices[1].tick(t0 + 3 * DAY).unwrap().is_empty());
    assert_eq!(w.devices[1].next_fire_at(), Some(watered + 3 * DAY));
    // Had it not heard, it fires, and the completion still wins when they meet.
    let mut w = World::new(2);
    let rid = w.devices[0]
        .create_countdown_reminder(
            "Plants",
            countdown(3, CountdownUnit::Days, None),
            None,
            Some(t0),
            t0,
        )
        .unwrap();
    w.sync_all();
    w.devices[0]
        .complete_expected(&rid, watered, watered)
        .unwrap();
    assert_eq!(w.devices[1].tick(t0 + 3 * DAY).unwrap().len(), 1);
    w.sync_all();
    for d in &w.devices {
        assert_eq!(occurrences(d, &rid), 1);
        assert!(d.state().occurrences.values().all(|o| !o.is_open()));
        assert_eq!(d.next_fire_at(), Some(watered + 3 * DAY));
    }
}

#[test]
fn editing_a_countdown_restarts_nothing_but_moves_the_next_firing() {
    let mut c = core_in(NY);
    let t0 = at(NY, "2026-10-01T08:00:00");
    let rid = c
        .create_countdown_reminder(
            "Plants",
            countdown(3, CountdownUnit::Days, None),
            None,
            Some(t0),
            t0,
        )
        .unwrap();
    c.edit_reminder(
        &rid,
        EditReminder {
            countdown: Some(countdown(5, CountdownUnit::Days, None)),
            ..Default::default()
        },
        t0 + 1,
    )
    .unwrap();
    assert_eq!(c.next_fire_at(), Some(t0 + 5 * DAY));
    // Bad countdowns and schedules on a countdown are refused.
    for edit in [
        EditReminder {
            countdown: Some(countdown(0, CountdownUnit::Days, None)),
            ..Default::default()
        },
        EditReminder {
            countdown: Some(countdown(1, CountdownUnit::Hours, Some("09:00"))),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            c.edit_reminder(&rid, edit, t0 + 2),
            Err(Error::BadCountdown(_))
        ));
    }
    assert!(c
        .create_countdown_reminder("x", countdown(0, CountdownUnit::Days, None), None, None, t0)
        .is_err());
    assert!(c
        .create_countdown_reminder("", countdown(1, CountdownUnit::Days, None), None, None, t0)
        .is_err());
    let history = c.state().history(&rid, Setting::Countdown);
    assert_eq!(history.len(), 2);
}

#[test]
fn countdown_events_need_a_reader_of_the_new_format() {
    let e = Event::CountdownReminderCreated {
        reminder_id: "r".into(),
        title: "t".into(),
        countdown: countdown(1, CountdownUnit::Days, None),
        zone: None,
        last_done: None,
    };
    assert_eq!(e.format(), 5);
    let edit = Event::ReminderEdited {
        reminder_id: "r".into(),
        hlc: Default::default(),
        change: hab_core::Change::Countdown(countdown(1, CountdownUnit::Days, None)),
    };
    assert_eq!(edit.format(), 5);
    assert_eq!(hab_core::FORMAT_VERSION, 6);
    // And they survive a restart: the state rebuilds the same.
    let dir = std::env::temp_dir().join(format!("hab-countdown-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("core.db");
    let t0 = at("UTC", "2026-10-01T08:00:00");
    let rid = {
        let mut c = Core::open(&path).unwrap();
        c.set_device_zone("UTC").unwrap();
        c.create_countdown_reminder(
            "Dose",
            countdown(2, CountdownUnit::Hours, None),
            None,
            Some(t0),
            t0,
        )
        .unwrap()
    };
    let c = Core::open(&path).unwrap();
    assert_eq!(
        c.state().reminders[&rid].countdown,
        Some(countdown(2, CountdownUnit::Hours, None))
    );
    assert_eq!(c.next_fire_at(), Some(t0 + 2 * HOUR));
    let _ = std::fs::remove_dir_all(&dir);
}
