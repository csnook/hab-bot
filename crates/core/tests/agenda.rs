//! The Agenda's data (#52): what it holds from yesterday to the end of
//! tomorrow, in the device's zone, and the remembered view.

use hab_core::{ClosingKind, Core, Error};

const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
// 2026-10-03T00:00:00Z
const D0: i64 = 1_790_985_600;

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

#[test]
fn it_covers_yesterday_today_and_tomorrow_and_nothing_beyond() {
    let mut c = core();
    let now = D0 + 12 * HOUR;
    for (title, at) in [
        ("Day before", D0 - DAY - HOUR),
        ("Tomorrow late", D0 + 2 * DAY - 60),
        ("Beyond", D0 + 2 * DAY + 60),
        ("Tonight", D0 + 20 * HOUR),
    ] {
        c.create_reminder(title, at, D0 - 3 * DAY).unwrap();
    }
    let a = c.agenda(now);
    assert_eq!(
        a.days,
        vec![(D0 - DAY, D0), (D0, D0 + DAY), (D0 + DAY, D0 + 2 * DAY)]
    );
    let titles: Vec<&str> = a.expected.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, vec!["Tonight", "Tomorrow late"]);
}

#[test]
fn closed_occurrences_from_yesterday_are_in_scheduled_order_and_older_ones_are_not() {
    let mut c = core();
    let old = c
        .create_reminder("Old", D0 - 2 * DAY + HOUR, D0 - 3 * DAY)
        .unwrap();
    let y = c
        .create_reminder("Yesterday", D0 - DAY + 2 * HOUR, D0 - 3 * DAY)
        .unwrap();
    let t = c.create_reminder("Today", D0 + HOUR, D0 - 3 * DAY).unwrap();
    c.tick(D0 - 2 * DAY + HOUR).unwrap();
    c.complete(
        &format!("{old}@{}", D0 - 2 * DAY + HOUR),
        D0 - 2 * DAY + 2 * HOUR,
    )
    .unwrap();
    c.tick(D0 - DAY + 2 * HOUR).unwrap();
    // Closed late today, scheduled yesterday: it sits at its scheduled time.
    c.tick(D0 + HOUR).unwrap();
    c.complete(&format!("{t}@{}", D0 + HOUR), D0 + 2 * HOUR)
        .unwrap();
    c.skip(&format!("{y}@{}", D0 - DAY + 2 * HOUR), None, D0 + 3 * HOUR)
        .unwrap();
    let a = c.agenda(D0 + 4 * HOUR);
    let got: Vec<(&str, ClosingKind)> = a
        .closed
        .iter()
        .map(|e| (e.title.as_str(), e.kind))
        .collect();
    assert_eq!(
        got,
        vec![
            ("Yesterday", ClosingKind::Skipped),
            ("Today", ClosingKind::Completed)
        ]
    );
}

#[test]
fn open_occurrences_are_the_inbox_s_and_the_days_follow_the_device_zone() {
    let mut c = core();
    let id = c.create_reminder("Open", D0 + HOUR, D0 - DAY).unwrap();
    c.tick(D0 + HOUR).unwrap();
    let now = D0 + 2 * HOUR;
    let a = c.agenda(now);
    let i = c.inbox(now);
    assert_eq!(a.due, i.due);
    assert_eq!(a.overdue, i.overdue);
    let open: Vec<_> = a.overdue.iter().chain(&a.due).collect();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].reminder_id, id);
    // Auckland is 13 hours ahead in October, so "today" starts earlier.
    c.set_device_zone("Pacific/Auckland").unwrap();
    let a = c.agenda(now);
    assert_ne!(a.days[1].0, D0);
    assert_eq!(a.days[1].1 - a.days[1].0, DAY);
}

#[test]
fn a_day_that_changes_clocks_is_not_24_hours() {
    let c = core();
    c.set_device_zone("America/New_York").unwrap();
    // 2026-11-01 is the day the clocks go back: 25 hours.
    let now = 1_793_534_400; // 2026-11-01T12:00:00Z
    let a = c.agenda(now);
    assert_eq!(a.days[1].1 - a.days[1].0, 25 * HOUR);
    assert_eq!(a.days[0].1, a.days[1].0);
    assert_eq!(a.days[1].1, a.days[2].0);
}

#[test]
fn the_view_is_remembered_on_the_device_and_inbox_the_first_time() {
    let c = core();
    assert_eq!(c.view(), "inbox");
    c.set_view("agenda").unwrap();
    assert_eq!(c.view(), "agenda");
    assert!(matches!(c.set_view("nope"), Err(Error::BadView(_))));
    assert_eq!(c.view(), "agenda");
}

#[test]
fn the_first_run_tip_is_shown_until_dismissed_and_help_shows_it_again() {
    let c = core();
    assert!(!c.view_tip_seen());
    c.set_view_tip_seen(true).unwrap();
    assert!(c.view_tip_seen());
    c.set_view_tip_seen(false).unwrap();
    assert!(!c.view_tip_seen());
}
