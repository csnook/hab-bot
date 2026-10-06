//! The check with the server before every alert (spec: Alerts → Alerting on
//! several devices), through the alerter's platform-independent seam.

use hab_core::{
    AlertStyle, Alerter, Command, Core, EditReminder, Priority, ServerCheck, ACTION_ACKNOWLEDGE,
};
use std::cell::Cell;

const MIN: i64 = 60;
const HOUR: i64 = 3_600;
const T0: i64 = 1_790_000_000;

/// A server the test controls: whether it can be reached, and which checks
/// have come back.
#[derive(Default)]
struct Server {
    unreachable: Cell<bool>,
    requests: Cell<u64>,
    answered: Cell<u64>,
}

impl Server {
    fn answer_all(&self) {
        self.answered.set(self.requests.get());
    }
}

impl ServerCheck for Server {
    fn reachable(&self) -> bool {
        !self.unreachable.get()
    }
    fn request(&self) -> u64 {
        self.requests.set(self.requests.get() + 1);
        self.requests.get()
    }
    fn done(&self, ticket: u64) -> bool {
        self.answered.get() >= ticket
    }
}

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

fn fired(c: &mut Core, p: Priority) -> String {
    let id = c.create_reminder("Medicine", T0, T0 - HOUR).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            priority: Some(p),
            ..Default::default()
        },
        T0 - HOUR,
    )
    .unwrap();
    c.tick(T0).unwrap();
    format!("{id}@{T0}")
}

fn shows(commands: &[Command]) -> usize {
    commands
        .iter()
        .filter(|c| matches!(c, Command::Show(_)))
        .count()
}

fn history(c: &Core) -> usize {
    c.state().alerts.len()
}

#[test]
fn an_alert_waits_for_the_server_and_goes_ahead_when_the_check_comes_back() {
    let (mut c, server, mut a) = (core(), Server::default(), Alerter::new());
    fired(&mut c, Priority::Medium);
    let p = a.pass_with(&mut c, T0, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 0);
    assert_eq!(server.requests.get(), 1);
    assert_eq!(history(&c), 0, "nothing is in the history until it alerts");
    // It looks again when the wait is up, whatever happens.
    assert_eq!(p.next_at, Some(T0 + 60));

    // Still waiting a moment later; it doesn't ask again.
    let p = a.pass_with(&mut c, T0 + 2, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 0);
    assert_eq!(server.requests.get(), 1);

    server.answer_all();
    let p = a.pass_with(&mut c, T0 + 3, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 1);
    assert_eq!(history(&c), 1);
    // A standing gentle alert doesn't ask again.
    let p = a.pass_with(&mut c, T0 + 4, false, &server).unwrap();
    assert!(p.commands.is_empty());
    assert_eq!(server.requests.get(), 1);
}

#[test]
fn the_wait_is_capped_by_the_priority_s_server_wait() {
    let (mut c, server, mut a) = (core(), Server::default(), Alerter::new());
    fired(&mut c, Priority::High);
    assert_eq!(Priority::High.settings().server_wait, Some(60));
    assert_eq!(
        shows(&a.pass_with(&mut c, T0, false, &server).unwrap().commands),
        0
    );
    // The server never answers.
    let p = a.pass_with(&mut c, T0 + 59, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 0);
    let p = a.pass_with(&mut c, T0 + 60, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 1);
    assert_eq!(history(&c), 1);
}

#[test]
fn a_shorter_server_wait_from_the_settings_caps_it() {
    fn five_seconds(p: Priority) -> hab_core::PrioritySettings {
        hab_core::PrioritySettings {
            server_wait: Some(5),
            ..p.settings()
        }
    }
    let (mut c, server) = (core(), Server::default());
    let mut a = Alerter::with_settings(five_seconds);
    fired(&mut c, Priority::Medium);
    let p = a.pass_with(&mut c, T0, false, &server).unwrap();
    assert_eq!(p.next_at, Some(T0 + 5));
    assert_eq!(
        shows(
            &a.pass_with(&mut c, T0 + 5, false, &server)
                .unwrap()
                .commands
        ),
        1
    );
}

#[test]
fn with_the_server_out_of_reach_the_device_alerts_without_waiting() {
    let (mut c, server, mut a) = (core(), Server::default(), Alerter::new());
    server.unreachable.set(true);
    fired(&mut c, Priority::Medium);
    let p = a.pass_with(&mut c, T0, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 1);
    assert_eq!(server.requests.get(), 0, "nothing to ask");
}

#[test]
fn losing_the_server_while_waiting_releases_the_alert() {
    let (mut c, server, mut a) = (core(), Server::default(), Alerter::new());
    fired(&mut c, Priority::Medium);
    assert_eq!(
        shows(&a.pass_with(&mut c, T0, false, &server).unwrap().commands),
        0
    );
    server.unreachable.set(true);
    assert_eq!(
        shows(
            &a.pass_with(&mut c, T0 + 1, false, &server)
                .unwrap()
                .commands
        ),
        1
    );
}

#[test]
fn maximum_rings_at_once_and_asks_the_server_meanwhile() {
    let (mut c, server, mut a) = (core(), Server::default(), Alerter::new());
    fired(&mut c, Priority::Maximum);
    let p = a.pass_with(&mut c, T0, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 1);
    assert_eq!(server.requests.get(), 1);
}

#[test]
fn every_repeat_checks_first_and_the_standing_alert_stays_meanwhile() {
    let (mut c, server, mut a) = (core(), Server::default(), Alerter::new());
    // Insistent once overdue: due for an hour, then repeats every 10 minutes.
    fired(&mut c, Priority::Medium);
    let overdue = T0 + HOUR;
    server.answer_all();
    // Due: gentle, a check came back at once.
    let _ = a.pass_with(&mut c, T0, false, &server).unwrap();
    server.answer_all();
    let p = a.pass_with(&mut c, T0 + 1, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 1);

    // It goes overdue: a change of style checks again.
    let p = a.pass_with(&mut c, overdue, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 0);
    let asked = server.requests.get();
    server.answer_all();
    let p = a.pass_with(&mut c, overdue + 1, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 1);

    // The repeat, 10 minutes after, checks first.
    let repeat = overdue + 1 + 10 * MIN;
    let p = a.pass_with(&mut c, repeat, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 0, "held for the check");
    assert!(
        !p.commands
            .iter()
            .any(|c| matches!(c, Command::Close { .. })),
        "the alert standing stays up while it waits"
    );
    assert_eq!(server.requests.get(), asked + 1);
    server.answer_all();
    let p = a.pass_with(&mut c, repeat + 1, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 1);
}

#[test]
fn an_occurrence_closed_while_the_alert_waited_is_never_alerted() {
    // The check brought in another device's completion: the next pass sees
    // the occurrence closed and says nothing.
    let (mut c, server, mut a) = (core(), Server::default(), Alerter::new());
    let occ = fired(&mut c, Priority::High);
    assert_eq!(
        shows(&a.pass_with(&mut c, T0, false, &server).unwrap().commands),
        0
    );
    c.complete(&occ, T0 + 1).unwrap();
    server.answer_all();
    let p = a.pass_with(&mut c, T0 + 2, false, &server).unwrap();
    assert!(p.commands.is_empty());
    assert_eq!(history(&c), 0);
}

#[test]
fn an_acknowledgement_that_arrives_while_waiting_stops_the_alert_and_a_later_one_asks_again() {
    let (mut c, server, mut a) = (core(), Server::default(), Alerter::new());
    let occ = fired(&mut c, Priority::High);
    let _ = a.pass_with(&mut c, T0, false, &server).unwrap();
    c.acknowledge(&occ, T0 + 1).unwrap();
    server.answer_all();
    let p = a.pass_with(&mut c, T0 + 2, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 0);
    // Quiet for one overdue interval, then it alerts, with a fresh check
    // rather than the old answer.
    let asked = server.requests.get();
    let later = T0 + 1 + 10 * MIN;
    let p = a.pass_with(&mut c, later, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 0);
    assert_eq!(server.requests.get(), asked + 1);
    server.answer_all();
    let p = a.pass_with(&mut c, later + 1, false, &server).unwrap();
    assert_eq!(shows(&p.commands), 1);
    let Command::Show(n) = &p.commands[0] else {
        panic!()
    };
    assert_eq!(n.style, AlertStyle::Alarm);
    assert!(n.actions.iter().any(|(k, _)| *k == ACTION_ACKNOWLEDGE));
}

#[test]
fn a_standalone_device_never_waits() {
    let (mut c, mut a) = (core(), Alerter::new());
    fired(&mut c, Priority::Medium);
    assert_eq!(shows(&a.pass(&mut c, T0, false).unwrap().commands), 1);
}
