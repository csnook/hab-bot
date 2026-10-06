//! Settings → This device (#51), through the core's own API and the
//! alerter's seams: the loudest alert style, "Quiet this device until…", and
//! the device's name and whether it is portable.

use hab_core::{
    AlertStyle, Alerter, Command, Core, DeviceQuiet, EditReminder, Error, Event, LoudestAlert,
    Notification, Priority, ServerCheck, SnoozeAllChoice, FORMAT_VERSION,
};
use std::cell::Cell;

const MIN: i64 = 60;
const HOUR: i64 = 3_600;
const T0: i64 = 1_790_000_000;

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

fn fired_with(c: &mut Core, p: Priority, expiry: Option<i64>) -> String {
    let id = c.create_reminder("Medicine", T0, T0 - HOUR).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            priority: Some(p),
            expiry: expiry.map(|d| vec![hab_core::Delay::After(d)]),
            ..Default::default()
        },
        T0 - HOUR,
    )
    .unwrap();
    c.tick(T0).unwrap();
    format!("{id}@{T0}")
}

fn fired(c: &mut Core, p: Priority) -> String {
    fired_with(c, p, None)
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

fn cap(c: &Core, style: AlertStyle, caps_maximum: bool) {
    c.set_loudest_alert(LoudestAlert {
        style,
        caps_maximum,
    })
    .unwrap();
}

fn history(c: &Core) -> Vec<AlertStyle> {
    c.state().alerts.iter().map(|a| a.style).collect()
}

#[test]
fn without_settings_nothing_is_limited() {
    let mut c = core();
    assert_eq!(c.loudest_alert(), LoudestAlert::default());
    assert_eq!(c.device_quiet(T0), None);
    fired(&mut c, Priority::High);
    let p = Alerter::new().pass(&mut c, T0, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);
}

#[test]
fn the_cap_downgrades_louder_styles_on_this_device() {
    let mut c = core();
    cap(&c, AlertStyle::Insistent, false);
    assert_eq!(c.loudest_alert().style, AlertStyle::Insistent);
    fired(&mut c, Priority::High);
    let mut a = Alerter::new();
    let p = a.pass(&mut c, T0, false).unwrap();
    let n = shown(&p.commands)[0].clone();
    assert_eq!(n.style, AlertStyle::Insistent);
    assert_eq!(n.urgency, hab_core::Urgency::Normal);
    // It repeats as an insistent alert does, not as an alarm.
    assert!(p.next_at.is_some());
    // The history records the style this device alerted in.
    assert_eq!(history(&c), [AlertStyle::Insistent]);
}

#[test]
fn a_cap_leaves_quieter_styles_alone() {
    let mut c = core();
    cap(&c, AlertStyle::Insistent, false);
    fired(&mut c, Priority::Low);
    let p = Alerter::new().pass(&mut c, T0, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Gentle);
}

#[test]
fn an_overdue_alarm_step_is_capped_too() {
    let mut c = core();
    cap(&c, AlertStyle::Gentle, false);
    fired(&mut c, Priority::Medium);
    let mut a = Alerter::new();
    let overdue = T0 + HOUR;
    a.pass(&mut c, T0, false).unwrap();
    // Medium reaches an alarm two hours after it goes overdue; here a gentle
    // cap holds it at gentle all the way, so only the style changes record.
    let p = a.pass(&mut c, overdue + 2 * HOUR, false).unwrap();
    assert!(shown(&p.commands)
        .iter()
        .all(|n| n.style == AlertStyle::Gentle));
    assert_eq!(history(&c), [AlertStyle::Gentle]);
}

#[test]
fn maximum_gets_through_the_cap_unless_the_cap_says_otherwise() {
    let mut c = core();
    cap(&c, AlertStyle::Gentle, false);
    fired(&mut c, Priority::Maximum);
    let p = Alerter::new().pass(&mut c, T0, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);

    let mut c = core();
    cap(&c, AlertStyle::Gentle, true);
    fired(&mut c, Priority::Maximum);
    let p = Alerter::new().pass(&mut c, T0, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Gentle);
}

#[test]
fn quiet_makes_everything_silent_until_it_ends_and_then_alerts_at_the_current_level() {
    let mut c = core();
    fired(&mut c, Priority::High);
    c.quiet_device(T0 + HOUR, false, T0).unwrap();
    assert_eq!(
        c.device_quiet(T0),
        Some(DeviceQuiet {
            until: T0 + HOUR,
            include_maximum: false
        })
    );
    let mut a = Alerter::new();
    let p = a.pass(&mut c, T0, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Silent);
    // It asks to be looked at again when the quiet ends.
    assert!(p.next_at.is_some_and(|t| t <= T0 + HOUR));
    // Silent is not repeated.
    let p = a.pass(&mut c, T0 + 30 * MIN, false).unwrap();
    assert!(shown(&p.commands).is_empty());
    // The quiet ends: the alert comes at its current level, once.
    assert_eq!(c.device_quiet(T0 + HOUR), None);
    let p = a.pass(&mut c, T0 + HOUR, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);
    assert_eq!(history(&c), [AlertStyle::Silent, AlertStyle::Alarm]);
}

#[test]
fn quiet_ending_early_brings_the_alert_back() {
    let mut c = core();
    fired(&mut c, Priority::High);
    c.quiet_device(T0 + HOUR, false, T0).unwrap();
    let mut a = Alerter::new();
    a.pass(&mut c, T0, false).unwrap();
    c.end_quiet_device().unwrap();
    assert_eq!(c.device_quiet(T0 + MIN), None);
    let p = a.pass(&mut c, T0 + MIN, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);
}

#[test]
fn quiet_leaves_out_maximum_unless_included() {
    let mut c = core();
    fired(&mut c, Priority::Maximum);
    c.quiet_device(T0 + HOUR, false, T0).unwrap();
    let p = Alerter::new().pass(&mut c, T0, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);

    let mut c = core();
    fired(&mut c, Priority::Maximum);
    c.quiet_device(T0 + HOUR, true, T0).unwrap();
    let p = Alerter::new().pass(&mut c, T0, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Silent);
}

#[test]
fn quiet_is_not_a_snooze() {
    let mut c = core();
    let occ = fired(&mut c, Priority::High);
    let before = c.unsent().unwrap().len();
    c.quiet_device(T0 + HOUR, true, T0).unwrap();
    cap(&c, AlertStyle::Gentle, true);
    c.end_quiet_device().unwrap();
    c.quiet_device_for(SnoozeAllChoice::Minutes(30), false, T0)
        .unwrap();
    // Nothing written to any list, nothing snoozed.
    assert_eq!(c.unsent().unwrap().len(), before);
    let inbox = c.inbox(T0 + MIN);
    assert!(inbox
        .overdue
        .iter()
        .chain(inbox.due.iter())
        .all(|d| d.snoozed_until.is_none()));
    assert!(c.state().occurrences[&occ].snoozed_until.is_none());
    Alerter::new().pass(&mut c, T0, false).unwrap();
    assert!(c.state().occurrences[&occ].snoozed_until.is_none());
}

#[test]
fn quiet_is_this_device_only_and_other_devices_alert_in_full() {
    let (mut mine, mut other) = (core(), core());
    fired(&mut mine, Priority::High);
    fired(&mut other, Priority::High);
    mine.quiet_device(T0 + HOUR, false, T0).unwrap();
    cap(&mine, AlertStyle::Gentle, false);
    let p = Alerter::new().pass(&mut mine, T0, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Silent);
    let p = Alerter::new().pass(&mut other, T0, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);
    assert_eq!(other.device_quiet(T0), None);
    assert_eq!(other.loudest_alert(), LoudestAlert::default());
}

#[test]
fn a_quiet_setting_that_has_passed_does_nothing_and_a_past_time_is_refused() {
    let mut c = core();
    assert!(matches!(
        c.quiet_device(T0, false, T0),
        Err(Error::SnoozeInThePast)
    ));
    c.quiet_device(T0 + MIN, false, T0).unwrap();
    fired(&mut c, Priority::High);
    let p = Alerter::new().pass(&mut c, T0 + HOUR, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);
}

#[test]
fn the_last_chance_alert_is_limited_like_any_other() {
    let mut c = core();
    let occ = fired_with(&mut c, Priority::High, Some(3 * HOUR));
    let mut a = Alerter::new();
    a.pass(&mut c, T0, false).unwrap();
    c.snooze(&occ, T0 + 24 * HOUR, T0 + MIN).unwrap();
    cap(&c, AlertStyle::Gentle, false);
    let chance = T0 + 3 * HOUR - 10 * MIN;
    let p = a.pass(&mut c, chance, false).unwrap();
    let n = shown(&p.commands)[0].clone();
    assert_eq!(n.title, "Last chance: Medicine");
    assert_eq!(n.style, AlertStyle::Gentle);

    // And on a quiet device it comes silently rather than not at all.
    let mut c = core();
    let occ = fired_with(&mut c, Priority::High, Some(3 * HOUR));
    let mut a = Alerter::new();
    a.pass(&mut c, T0, false).unwrap();
    c.snooze(&occ, T0 + 24 * HOUR, T0 + MIN).unwrap();
    c.quiet_device(T0 + 4 * HOUR, false, T0 + MIN).unwrap();
    let p = a.pass(&mut c, chance, false).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Silent);
}

#[derive(Default)]
struct Server {
    requests: Cell<u64>,
}

impl ServerCheck for Server {
    fn reachable(&self) -> bool {
        true
    }
    fn request(&self) -> u64 {
        self.requests.set(self.requests.get() + 1);
        self.requests.get()
    }
    fn done(&self, _ticket: u64) -> bool {
        false
    }
}

#[test]
fn a_limited_alert_still_waits_for_the_server_check_within_the_priority_s_wait() {
    let mut c = core();
    fired(&mut c, Priority::High);
    c.quiet_device(T0 + HOUR, false, T0).unwrap();
    let (server, mut a) = (Server::default(), Alerter::new());
    let wait = Priority::High.settings().server_wait.unwrap();
    let p = a.pass_with(&mut c, T0, false, &server).unwrap();
    assert!(shown(&p.commands).is_empty(), "waits for the check");
    assert_eq!(server.requests.get(), 1);
    let p = a.pass_with(&mut c, T0 + wait, false, &server).unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Silent);
    // Maximum is exempt from the cap and rings at once, asking meanwhile.
    let mut c = core();
    fired(&mut c, Priority::Maximum);
    cap(&c, AlertStyle::Silent, false);
    let server = Server::default();
    let p = Alerter::new()
        .pass_with(&mut c, T0, false, &server)
        .unwrap();
    assert_eq!(shown(&p.commands)[0].style, AlertStyle::Alarm);
    assert_eq!(server.requests.get(), 1);
}

#[test]
fn the_cap_and_quiet_survive_signing_in_and_never_sync() {
    let mut c = core();
    cap(&c, AlertStyle::Gentle, true);
    c.quiet_device(T0 + HOUR, true, T0).unwrap();
    c.join("u1", "7").unwrap();
    c.link_account("account-list", "Laptop", T0).unwrap();
    assert_eq!(c.loudest_alert().style, AlertStyle::Gentle);
    assert!(c.loudest_alert().caps_maximum);
    assert!(c.device_quiet(T0).is_some());
    // Nothing of either is an event.
    for o in c.unsent().unwrap() {
        let text = String::from_utf8_lossy(&o.payload).to_string();
        assert!(
            !text.contains("quiet") && !text.contains("loudest"),
            "{text}"
        );
    }
}

#[test]
fn the_name_and_portability_are_personal_events_that_other_devices_read() {
    let mut a = Core::open_in_memory().unwrap();
    a.join("u1", "1").unwrap();
    let list = a.personal_list_id().to_string();
    let mut b = Core::open_in_memory().unwrap();
    b.join("u1", "2").unwrap();
    b.use_personal_list(&list).unwrap();

    a.name_device("Desk", T0).unwrap();
    a.set_portable(true, T0).unwrap();
    // Saying the same again records nothing.
    let n = a.unsent().unwrap().len();
    a.set_portable(true, T0 + 1).unwrap();
    assert_eq!(a.unsent().unwrap().len(), n);
    assert_eq!(a.device_portable("1"), Some(true));
    assert_eq!(a.own_device_name(), Some("Desk"));

    for (i, o) in a.unsent().unwrap().into_iter().enumerate() {
        assert_eq!(o.list_id, list);
        b.receive(&list, i as i64 + 1, &o.event_id, "1", o.format, &o.payload)
            .unwrap();
    }
    assert_eq!(b.device_portable("1"), Some(true));
    assert_eq!(b.device_name("1"), Some("Desk"));
    assert_eq!(b.device_portable("2"), None);

    a.set_portable(false, T0 + 2).unwrap();
    assert_eq!(a.device_portable("1"), Some(false));
    assert_eq!(Event::DevicePortable { portable: true }.format(), 13);
    assert_eq!(FORMAT_VERSION, 13);
}
