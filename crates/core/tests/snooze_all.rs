//! Snooze all and quiet hours (#50, ADR 0013), through the core's own API:
//! what they hold, what breaks through, how they end, what they record, and
//! how several devices of one user agree.

use hab_core::{
    Alerter, Command, Core, Delay, EditReminder, Error, Event, Priority, QuietHours, Scope,
    SnoozeEnd, Source, FORMAT_VERSION,
};

const MIN: i64 = 60;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
/// 2026-09-21 14:13:20 UTC, a Monday.
const T0: i64 = 1_790_000_000;
/// Midnight at the start of that Monday.
const MONDAY: i64 = T0 - (14 * HOUR + 13 * MIN + 20);
const USER: &str = "u1";

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

fn reminder_in(
    c: &mut Core,
    list: &str,
    title: &str,
    p: Priority,
    at: i64,
    expiry: Option<i64>,
) -> String {
    let id = c.create_reminder_in(list, title, at, MONDAY).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            priority: Some(p),
            expiry: expiry.map(|d| vec![Delay::After(d)]),
            ..Default::default()
        },
        MONDAY,
    )
    .unwrap();
    id
}

fn reminder(c: &mut Core, title: &str, p: Priority, at: i64) -> String {
    reminder_exp(c, title, p, at, None)
}

fn reminder_exp(c: &mut Core, title: &str, p: Priority, at: i64, expiry: Option<i64>) -> String {
    let list = c.personal_list_id().to_string();
    reminder_in(c, &list, title, p, at, expiry)
}

/// The titles an alerter shows at `now`.
fn alerts(a: &mut Alerter, c: &mut Core, now: i64) -> Vec<String> {
    let mut v: Vec<String> = a
        .pass(c, now, false)
        .unwrap()
        .commands
        .iter()
        .filter_map(|c| match c {
            Command::Show(n) => Some(n.title.clone()),
            _ => None,
        })
        .collect();
    v.sort();
    v
}

fn weeknights() -> QuietHours {
    QuietHours {
        days: ["MO", "TU", "WE", "TH", "FR"].map(String::from).to_vec(),
        from: "22:00".into(),
        to: "07:00".into(),
        scope: Scope::All,
        include_maximum: false,
    }
}

// ---- Snooze all ----

#[test]
fn it_holds_what_is_open_and_what_fires_before_it_ends() {
    let mut c = core();
    let mut a = Alerter::new();
    reminder(&mut c, "Open now", Priority::High, T0);
    c.tick(T0).unwrap();
    assert_eq!(alerts(&mut a, &mut c, T0), ["Open now"]);

    reminder(&mut c, "Fires later", Priority::Low, T0 + 30 * MIN);
    reminder(&mut c, "After the end", Priority::Low, T0 + 3 * HOUR);
    let until = T0 + 2 * HOUR;
    c.snooze_all(Scope::All, until, false, T0 + MIN).unwrap();

    // The alert standing for the open one comes down, and nothing alerts.
    let pass = a.pass(&mut c, T0 + MIN, false).unwrap();
    assert!(pass
        .commands
        .iter()
        .any(|c| matches!(c, Command::Close { .. })));
    assert!(!pass.commands.iter().any(|c| matches!(c, Command::Show(_))));
    assert_eq!(pass.next_at, Some(until));

    // One that fires inside the snooze fires quietly.
    assert_eq!(c.tick(T0 + 30 * MIN).unwrap().len(), 1);
    assert!(alerts(&mut a, &mut c, T0 + 30 * MIN).is_empty());
    let inbox = c.inbox(T0 + 30 * MIN);
    assert!(inbox
        .overdue
        .iter()
        .chain(&inbox.due)
        .all(|d| d.snoozed_until == Some(until)));

    // When it ends, both alert at their current level, once.
    assert_eq!(
        alerts(&mut a, &mut c, until),
        ["Fires later", "Open now"].map(String::from)
    );
    // One that fires after it ended is not held.
    c.tick(T0 + 3 * HOUR).unwrap();
    assert!(alerts(&mut a, &mut c, T0 + 3 * HOUR).contains(&"After the end".to_string()));
}

#[test]
fn maximum_is_left_out_unless_included() {
    for include in [false, true] {
        let mut c = core();
        let mut a = Alerter::new();
        reminder(&mut c, "Max", Priority::Maximum, T0);
        reminder(&mut c, "High", Priority::High, T0);
        c.tick(T0).unwrap();
        c.snooze_all(Scope::All, T0 + HOUR, include, T0 - 1)
            .unwrap();
        let shown = alerts(&mut a, &mut c, T0);
        assert_eq!(
            shown,
            if include {
                vec![]
            } else {
                vec!["Max".to_string()]
            }
        );
    }
}

#[test]
fn one_list_leaves_the_others_alone() {
    let mut c = core();
    let mut a = Alerter::new();
    let home = c.create_list("Home", None, MONDAY).unwrap();
    let personal = c.personal_list_id().to_string();
    reminder_in(&mut c, &home, "Bins", Priority::High, T0, None);
    reminder_in(&mut c, &personal, "Pills", Priority::High, T0, None);
    c.tick(T0).unwrap();
    c.snooze_all(Scope::List(home.clone()), T0 + HOUR, false, T0 - 1)
        .unwrap();
    assert_eq!(alerts(&mut a, &mut c, T0), ["Pills"]);
    let chips = c.holding(T0);
    assert_eq!(chips.len(), 1);
    assert_eq!(chips[0].list_name.as_deref(), Some("Home"));
}

#[test]
fn occurrences_still_go_overdue_on_schedule_and_last_chance_comes() {
    let mut c = core();
    let mut a = Alerter::new();
    let id = reminder_exp(&mut c, "Plumber", Priority::Medium, T0, Some(2 * HOUR));
    c.tick(T0).unwrap();
    let occ = format!("{id}@{T0}");
    let until = T0 + 3 * HOUR;
    c.snooze_all(Scope::All, until, false, T0 + MIN).unwrap();
    assert!(alerts(&mut a, &mut c, T0 + MIN).is_empty());

    // It goes overdue (Medium: after an hour) while held, quietly.
    let overdue_at = T0 + HOUR;
    assert!(c
        .inbox(overdue_at)
        .overdue
        .iter()
        .any(|d| d.occurrence_id == occ));
    assert!(alerts(&mut a, &mut c, overdue_at).is_empty());

    // The expiry is at T0+2h, inside the snooze: the last-chance alert comes
    // 10 minutes before, in the priority's style and never quieter than gentle.
    let chance = T0 + 2 * HOUR - 10 * MIN;
    assert!(alerts(&mut a, &mut c, chance - 1).is_empty());
    let pass = a.pass(&mut c, chance, false).unwrap();
    let n = pass
        .commands
        .iter()
        .find_map(|c| match c {
            Command::Show(n) => Some(n.clone()),
            _ => None,
        })
        .expect("a last-chance alert");
    assert_eq!(n.title, "Last chance: Plumber");
    assert!(n.body.starts_with("Expires at"));
    // The occurrence is still open and held until then.
    assert_eq!(c.inbox(chance).overdue[0].snoozed_until, Some(until));
}

#[test]
fn a_snooze_all_made_after_the_last_chance_moment_still_gets_one() {
    let mut c = core();
    let mut a = Alerter::new();
    reminder_exp(&mut c, "Tickets", Priority::Low, T0, Some(HOUR));
    c.tick(T0).unwrap();
    // 5 minutes before the expiry the user snoozes everything for 2 hours:
    // the dialog can't show this one's expiry, so the alert still comes.
    let now = T0 + HOUR - 5 * MIN;
    c.snooze_all(Scope::All, now + 2 * HOUR, false, now)
        .unwrap();
    assert_eq!(alerts(&mut a, &mut c, now), ["Last chance: Tickets"]);
}

#[test]
fn ending_it_early_alerts_everything_held_back() {
    let mut c = core();
    let mut a = Alerter::new();
    reminder(&mut c, "A", Priority::High, T0);
    reminder(&mut c, "B", Priority::Low, T0 + 10 * MIN);
    c.tick(T0).unwrap();
    let id = c.snooze_all(Scope::All, T0 + 4 * HOUR, false, T0).unwrap();
    assert!(alerts(&mut a, &mut c, T0).is_empty());
    c.tick(T0 + 10 * MIN).unwrap();
    assert!(alerts(&mut a, &mut c, T0 + 10 * MIN).is_empty());
    assert_eq!(c.holding(T0 + 20 * MIN).len(), 1);

    c.end_snooze_all(&id, T0 + 20 * MIN).unwrap();
    assert!(c.holding(T0 + 20 * MIN).is_empty());
    assert_eq!(alerts(&mut a, &mut c, T0 + 20 * MIN), ["A", "B"]);
    // Ending it again, or one that is over, does nothing; an unknown one fails.
    c.end_snooze_all(&id, T0 + 21 * MIN).unwrap();
    assert!(matches!(
        c.end_snooze_all("nope", T0),
        Err(Error::NoSnoozeAll(_))
    ));
}

#[test]
fn a_new_snooze_all_of_the_same_scope_replaces_the_old() {
    let mut c = core();
    reminder(&mut c, "A", Priority::High, T0);
    c.tick(T0).unwrap();
    c.snooze_all(Scope::All, T0 + 4 * HOUR, false, T0).unwrap();
    c.snooze_all(Scope::All, T0 + HOUR, false, T0 + MIN)
        .unwrap();
    let held = c.holding(T0 + 2 * MIN);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].until, T0 + HOUR);
    assert_eq!(
        c.inbox(T0 + 2 * MIN).overdue[0].snoozed_until,
        Some(T0 + HOUR)
    );
    // A list's does not replace everything's.
    let home = c.create_list("Home", None, MONDAY).unwrap();
    c.snooze_all(Scope::List(home), T0 + 2 * HOUR, false, T0 + 3 * MIN)
        .unwrap();
    assert_eq!(c.holding(T0 + 4 * MIN).len(), 2);
}

#[test]
fn a_single_snooze_that_lasts_longer_wins() {
    let mut c = core();
    let id = reminder(&mut c, "A", Priority::High, T0);
    c.tick(T0).unwrap();
    let occ = format!("{id}@{T0}");
    c.snooze_all(Scope::All, T0 + HOUR, false, T0).unwrap();
    c.snooze(&occ, T0 + 3 * HOUR, T0 + MIN).unwrap();
    assert_eq!(
        c.inbox(T0 + 2 * MIN).overdue[0].snoozed_until,
        Some(T0 + 3 * HOUR)
    );
    // Ending the snooze-all leaves the occurrence's own snooze.
    let sa = c.holding(T0 + 2 * MIN)[0].id.clone().unwrap();
    c.end_snooze_all(&sa, T0 + 5 * MIN).unwrap();
    assert_eq!(
        c.inbox(T0 + 6 * MIN).overdue[0].snoozed_until,
        Some(T0 + 3 * HOUR)
    );
}

#[test]
fn bad_snooze_alls_and_quiet_hours_are_refused() {
    let mut c = core();
    assert!(matches!(
        c.snooze_all(Scope::All, T0, false, T0),
        Err(Error::SnoozeInThePast)
    ));
    assert!(matches!(
        c.snooze_all(Scope::List("nope".into()), T0 + HOUR, false, T0),
        Err(Error::NoList(_))
    ));
    let mut q = weeknights();
    q.days.clear();
    assert!(matches!(
        c.set_quiet_hours(vec![q], T0),
        Err(Error::BadQuietHours(_))
    ));
    assert!(c.quiet_hours().is_empty());
}

// ---- Recording ----

#[test]
fn each_affected_occurrence_records_a_snooze_marked_as_part_of_a_snooze_all() {
    let mut c = core();
    let a = reminder(&mut c, "A", Priority::High, T0);
    let b = reminder(&mut c, "B", Priority::High, T0 + 10 * MIN);
    let max = reminder(&mut c, "Max", Priority::Maximum, T0);
    c.tick(T0).unwrap();
    let id = c
        .snooze_all(Scope::All, T0 + 2 * HOUR, false, T0 + MIN)
        .unwrap();
    c.tick(T0 + 10 * MIN).unwrap();
    let (oa, ob, omax) = (
        format!("{a}@{T0}"),
        format!("{b}@{}", T0 + 10 * MIN),
        format!("{max}@{T0}"),
    );

    let h = c.snooze_history(&oa, T0 + 20 * MIN);
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].via, Some(Source::SnoozeAll));
    assert_eq!(
        (h[0].set_at, h[0].until, h[0].ended),
        (T0 + MIN, T0 + 2 * HOUR, None)
    );
    // The one that fired inside it starts holding when it fired.
    let h = c.snooze_history(&ob, T0 + 20 * MIN);
    assert_eq!(
        (h[0].set_at, h[0].via),
        (T0 + 10 * MIN, Some(Source::SnoozeAll))
    );
    // Maximum was left out: no snooze.
    assert!(c.snooze_history(&omax, T0 + 20 * MIN).is_empty());

    // It ran out for A; B was done meanwhile; and a third was ended early.
    c.complete(&ob, T0 + 30 * MIN).unwrap();
    let h = c.snooze_history(&ob, T0 + 3 * HOUR);
    assert_eq!(h[0].ended, Some(SnoozeEnd::Closed));
    assert_eq!(h[0].ended_at, Some(T0 + 30 * MIN));
    let h = c.snooze_history(&oa, T0 + 3 * HOUR);
    assert_eq!(
        (h[0].ended, h[0].ended_at),
        (Some(SnoozeEnd::Elapsed), Some(T0 + 2 * HOUR))
    );
    c.end_snooze_all(&id, T0 + 40 * MIN).unwrap();
    let h = c.snooze_history(&oa, T0 + 3 * HOUR);
    assert_eq!(
        (h[0].ended, h[0].ended_at),
        (Some(SnoozeEnd::Ended), Some(T0 + 40 * MIN))
    );
    // A snooze made on the occurrence itself is recorded alongside, unmarked.
    c.snooze(&oa, T0 + 4 * HOUR, T0 + 50 * MIN).unwrap();
    let h = c.snooze_history(&oa, T0 + 51 * MIN);
    assert_eq!(h.len(), 2);
    assert_eq!(h[1].via, None);
}

// ---- Quiet hours ----

#[test]
fn quiet_hours_hold_what_is_open_and_what_fires_in_the_night() {
    let mut c = core();
    let mut a = Alerter::new();
    c.set_quiet_hours(vec![weeknights()], MONDAY).unwrap();
    let evening = MONDAY + 21 * HOUR;
    let night = MONDAY + 22 * HOUR;
    let late = MONDAY + 23 * HOUR;
    let morning = MONDAY + DAY + 7 * HOUR;
    reminder(&mut c, "Evening", Priority::Medium, evening);
    reminder(&mut c, "Night", Priority::Medium, late);
    c.tick(evening).unwrap();
    // The wake-up before 22:00 is the start of the quiet hours.
    let pass = a.pass(&mut c, evening, false).unwrap();
    assert!(pass.commands.iter().any(|c| matches!(c, Command::Show(_))));
    assert!(pass.next_at.unwrap() <= night);
    assert_eq!(c.next_quiet_boundary(evening), Some(night));

    // At 22:00 the standing alert comes down.
    let pass = a.pass(&mut c, night, false).unwrap();
    assert!(pass
        .commands
        .iter()
        .any(|c| matches!(c, Command::Close { .. })));
    // Something that fires at 23:00 is quiet.
    c.tick(late).unwrap();
    assert!(alerts(&mut a, &mut c, late).is_empty());
    assert_eq!(c.holding(late)[0].source, Source::QuietHours);
    assert_eq!(c.holding(late)[0].until, morning);
    // At 07:00 both alert.
    assert_eq!(alerts(&mut a, &mut c, morning), ["Evening", "Night"]);
}

#[test]
fn quiet_hours_leave_out_maximum_unless_included_and_can_cover_one_list() {
    let mut c = core();
    let mut a = Alerter::new();
    let home = c.create_list("Home", None, MONDAY).unwrap();
    let personal = c.personal_list_id().to_string();
    let t = MONDAY + 23 * HOUR;
    reminder_in(&mut c, &personal, "Max", Priority::Maximum, t, None);
    reminder_in(&mut c, &personal, "Personal", Priority::High, t, None);
    reminder_in(&mut c, &home, "Chore", Priority::High, t, None);
    c.tick(t).unwrap();

    let mut q = weeknights();
    q.scope = Scope::List(home.clone());
    c.set_quiet_hours(vec![q.clone()], MONDAY).unwrap();
    assert_eq!(alerts(&mut a, &mut c, t), ["Max", "Personal"]);

    let mut c2 = core();
    let mut a2 = Alerter::new();
    reminder(&mut c2, "Max", Priority::Maximum, t);
    reminder(&mut c2, "High", Priority::High, t);
    c2.tick(t).unwrap();
    q.scope = Scope::All;
    c2.set_quiet_hours(vec![q.clone()], MONDAY).unwrap();
    assert_eq!(alerts(&mut a2, &mut c2, t), ["Max"]);
    q.include_maximum = true;
    c2.set_quiet_hours(vec![q], MONDAY + 1).unwrap();
    // Included now: Maximum is held too (its alert standing comes down).
    let pass = a2.pass(&mut c2, t + 1, false).unwrap();
    assert!(pass
        .commands
        .iter()
        .any(|c| matches!(c, Command::Close { .. })));
}

#[test]
fn the_night_from_friday_runs_into_saturday_but_saturday_night_is_free() {
    let mut c = core();
    let mut a = Alerter::new();
    c.set_quiet_hours(vec![weeknights()], MONDAY).unwrap();
    let fri_night = MONDAY + 4 * DAY + 23 * HOUR;
    let sat_night = MONDAY + 5 * DAY + 23 * HOUR;
    reminder(&mut c, "Fri", Priority::High, fri_night);
    reminder(&mut c, "Sat", Priority::High, sat_night);
    c.tick(fri_night).unwrap();
    assert!(alerts(&mut a, &mut c, fri_night).is_empty());
    // Held to 07:00 on Saturday, when it alerts.
    assert!(alerts(&mut a, &mut c, MONDAY + 5 * DAY + 6 * HOUR + 59 * MIN).is_empty());
    assert_eq!(alerts(&mut a, &mut c, MONDAY + 5 * DAY + 7 * HOUR), ["Fri"]);
    // Saturday night is free: no hold, so what fires then alerts.
    c.tick(sat_night).unwrap();
    assert!(c.holding(sat_night).is_empty());
    assert!(alerts(&mut a, &mut c, sat_night).contains(&"Sat".to_string()));
}

#[test]
fn snoozes_made_by_quiet_hours_are_recorded_as_such() {
    let mut c = core();
    c.set_quiet_hours(vec![weeknights()], MONDAY).unwrap();
    let t = MONDAY + 23 * HOUR;
    let id = reminder(&mut c, "Night", Priority::High, t);
    c.tick(t).unwrap();
    let occ = format!("{id}@{t}");
    let h = c.snooze_history(&occ, t + MIN);
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].via, Some(Source::QuietHours));
    assert_eq!((h[0].set_at, h[0].until), (t, MONDAY + DAY + 7 * HOUR));
    assert_eq!(h[0].ended, None);
    // Done at 02:00: the hold ended when it closed.
    c.complete(&occ, MONDAY + DAY + 2 * HOUR).unwrap();
    let h = c.snooze_history(&occ, MONDAY + 2 * DAY);
    assert_eq!(h[0].ended, Some(SnoozeEnd::Closed));
    // One left open runs through the next night too.
    let t2 = MONDAY + DAY + 8 * HOUR;
    let id2 = reminder(&mut c, "Open", Priority::High, t2);
    c.tick(t2).unwrap();
    let occ2 = format!("{id2}@{t2}");
    let h = c.snooze_history(&occ2, MONDAY + 3 * DAY);
    let nights: Vec<_> = h.iter().map(|v| (v.set_at, v.via)).collect();
    assert_eq!(
        nights,
        vec![
            (MONDAY + DAY + 22 * HOUR, Some(Source::QuietHours)),
            (MONDAY + 2 * DAY + 22 * HOUR, Some(Source::QuietHours)),
        ]
    );
}

#[test]
fn the_last_chance_alert_comes_through_quiet_hours() {
    let mut c = core();
    let mut a = Alerter::new();
    c.set_quiet_hours(vec![weeknights()], MONDAY).unwrap();
    let t = MONDAY + 21 * HOUR + 30 * MIN;
    reminder_exp(&mut c, "Gate", Priority::Low, t, Some(2 * HOUR + 30 * MIN));
    c.tick(t).unwrap();
    // Expires at midnight, inside the night: 23:50 it alerts, though quiet.
    assert!(alerts(&mut a, &mut c, MONDAY + 23 * HOUR).is_empty());
    assert_eq!(
        alerts(&mut a, &mut c, MONDAY + 23 * HOUR + 50 * MIN),
        ["Last chance: Gate"]
    );
}

// ---- Several devices ----

/// Devices of one user and the server's numbering of their events.
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
        for d in &devices {
            d.set_device_zone("UTC").unwrap();
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

    fn sync_all(&mut self) {
        for i in 1..=self.devices.len() {
            self.upload(i);
        }
        for i in 1..=self.devices.len() {
            self.download(i);
        }
    }
}

#[test]
fn a_snooze_all_on_one_device_quiets_the_others_and_ending_it_there_alerts_here() {
    let mut w = World::new(2);
    let id = reminder(w.dev(1), "Bins", Priority::High, T0);
    w.sync_all();
    for i in [1, 2] {
        w.dev(i).tick(T0).unwrap();
    }
    w.sync_all();
    let (mut a1, mut a2) = (Alerter::new(), Alerter::new());
    // Device 1 snoozes all; device 2 hasn't heard: it alerts.
    let sa = w
        .dev(1)
        .snooze_all(Scope::All, T0 + 2 * HOUR, false, T0 + MIN)
        .unwrap();
    assert_eq!(alerts(&mut a2, w.dev(2), T0 + MIN), ["Bins"]);
    w.sync_all();
    // Having heard, it goes quiet, and the occurrence has one snooze record.
    let pass = a2.pass(w.dev(2), T0 + 2 * MIN, false).unwrap();
    assert!(pass
        .commands
        .iter()
        .any(|c| matches!(c, Command::Close { .. })));
    assert!(alerts(&mut a1, w.dev(1), T0 + 2 * MIN).is_empty());
    let occ = format!("{id}@{T0}");
    for i in [1, 2] {
        let h = w.dev(i).snooze_history(&occ, T0 + 3 * MIN);
        assert_eq!(h.len(), 1, "device {i}");
        assert_eq!(h[0].via, Some(Source::SnoozeAll));
        assert_eq!(w.dev(i).holding(T0 + 3 * MIN).len(), 1);
    }
    // Device 2 ends it; device 1 alerts once it has heard.
    w.dev(2).end_snooze_all(&sa, T0 + 10 * MIN).unwrap();
    assert!(alerts(&mut a1, w.dev(1), T0 + 11 * MIN).is_empty());
    w.sync_all();
    assert_eq!(alerts(&mut a1, w.dev(1), T0 + 12 * MIN), ["Bins"]);
    assert_eq!(alerts(&mut a2, w.dev(2), T0 + 12 * MIN), ["Bins"]);
}

#[test]
fn snooze_alls_made_on_two_devices_hold_together_and_either_can_end_its_own() {
    let mut w = World::new(2);
    reminder(w.dev(1), "A", Priority::High, T0);
    w.sync_all();
    for i in [1, 2] {
        w.dev(i).tick(T0).unwrap();
    }
    let s1 = w
        .dev(1)
        .snooze_all(Scope::All, T0 + HOUR, false, T0 + MIN)
        .unwrap();
    let s2 = w
        .dev(2)
        .snooze_all(Scope::All, T0 + 3 * HOUR, false, T0 + 2 * MIN)
        .unwrap();
    w.sync_all();
    for i in [1, 2] {
        assert_eq!(w.dev(i).holding(T0 + 3 * MIN).len(), 2, "device {i}");
        // Held until the later one ends.
        assert_eq!(
            w.dev(i).inbox(T0 + 3 * MIN).overdue[0].snoozed_until,
            Some(T0 + 3 * HOUR)
        );
    }
    w.dev(1).end_snooze_all(&s2, T0 + 5 * MIN).unwrap();
    w.sync_all();
    for i in [1, 2] {
        assert_eq!(
            w.dev(i).inbox(T0 + 6 * MIN).overdue[0].snoozed_until,
            Some(T0 + HOUR)
        );
    }
    let _ = s1;
}

#[test]
fn quiet_hours_set_on_two_devices_out_of_touch_converge_on_the_later() {
    let mut w = World::new(2);
    let mut a = weeknights();
    a.from = "21:00".into();
    let mut b = weeknights();
    b.to = "06:00".into();
    w.dev(1).set_quiet_hours(vec![a], T0).unwrap();
    w.dev(2).set_quiet_hours(vec![b.clone()], T0 + 5).unwrap();
    w.sync_all();
    for i in [1, 2] {
        assert_eq!(w.dev(i).quiet_hours(), vec![b.clone()], "device {i}");
    }
    // And clearing is a setting like any other.
    w.dev(1).set_quiet_hours(vec![], T0 + 10).unwrap();
    w.sync_all();
    for i in [1, 2] {
        assert!(w.dev(i).quiet_hours().is_empty());
    }
}

#[test]
fn quiet_hours_set_while_standalone_survive_a_restart() {
    let dir = std::env::temp_dir().join(format!("hab-quiet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("core.db");
    {
        let mut c = Core::open(&path).unwrap();
        c.set_quiet_hours(vec![weeknights()], T0).unwrap();
        c.snooze_all(Scope::All, T0 + HOUR, true, T0).unwrap();
    }
    let c = Core::open(&path).unwrap();
    assert_eq!(c.quiet_hours(), vec![weeknights()]);
    assert_eq!(c.holding(T0 + MIN).len(), 1);
    assert!(c.holding(T0 + MIN)[0].include_maximum);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- Format ----

#[test]
fn old_apps_keep_these_events_without_applying_them() {
    let d = Event::SnoozeAllStarted {
        snooze: hab_core::SnoozeAll {
            id: "s".into(),
            scope: Scope::All,
            include_maximum: false,
            from: 1,
            until: 2,
        },
    };
    assert_eq!(d.format(), 12);
    assert_eq!(
        Event::SnoozeAllEnded {
            snooze_id: "s".into()
        }
        .format(),
        12
    );
    assert_eq!(
        Event::QuietHoursSet {
            hlc: Default::default(),
            rules: vec![]
        }
        .format(),
        12
    );
    assert_eq!(FORMAT_VERSION, 12);
    // The wire form is stable and readable back.
    let json = serde_json::to_string(&d).unwrap();
    assert!(json.contains("\"type\":\"snooze_all_started\""), "{json}");
    assert!(json.contains("\"kind\":\"all\""), "{json}");
    assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), d);
}
