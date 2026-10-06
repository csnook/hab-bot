//! Priorities, going overdue and expiry delays, through the core's own API.

use hab_core::{
    built_in_priorities, AlertStyle, Change, ClosingKind, Core, Delay, DueItem, EditReminder,
    Error, Event, Hlc, Priority, Setting, State, StoredEvent,
};

const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
const T0: i64 = 1_790_000_000;

fn core() -> Core {
    let c = Core::open_in_memory().unwrap();
    c.set_device_zone("UTC").unwrap();
    c
}

fn with_priority(c: &mut Core, title: &str, fire_at: i64, p: Priority) -> String {
    let id = c.create_reminder(title, fire_at, T0 - DAY).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            priority: Some(p),
            ..Default::default()
        },
        T0 - DAY,
    )
    .unwrap();
    id
}

fn titles(items: &[DueItem]) -> Vec<&str> {
    items.iter().map(|d| d.title.as_str()).collect()
}

#[test]
fn the_built_in_priorities_carry_every_setting_in_the_spec_table() {
    let all = built_in_priorities();
    let names: Vec<_> = all.iter().map(|p| p.name).collect();
    assert_eq!(names, ["Minimum", "Low", "Medium", "High", "Maximum"]);
    let s = |p: Priority| p.settings();

    assert_eq!(s(Priority::Minimum).due_style, AlertStyle::Silent);
    assert_eq!(s(Priority::Low).due_style, AlertStyle::Gentle);
    assert_eq!(s(Priority::Medium).due_style, AlertStyle::Gentle);
    assert_eq!(s(Priority::High).due_style, AlertStyle::Alarm);
    assert_eq!(s(Priority::Maximum).due_style, AlertStyle::Alarm);

    let due: Vec<i64> = all.iter().map(|p| p.settings.due_interval).collect();
    assert_eq!(due, [DAY, DAY, HOUR, 0, 0]);
    let overdue: Vec<i64> = all.iter().map(|p| p.settings.overdue_interval).collect();
    assert_eq!(overdue, [DAY, DAY, 600, 600, 600]);

    let medium = s(Priority::Medium).overdue_steps;
    assert_eq!(medium.len(), 2);
    assert_eq!(
        (medium[0].after, medium[0].style),
        (0, AlertStyle::Insistent)
    );
    assert_eq!(
        (medium[1].after, medium[1].style),
        (HOUR, AlertStyle::Alarm)
    );
    assert_eq!(
        s(Priority::Minimum).overdue_steps[0].style,
        AlertStyle::Silent
    );
    assert_eq!(s(Priority::High).overdue_steps[0].style, AlertStyle::Alarm);

    let swipe: Vec<bool> = all.iter().map(|p| p.settings.swipeable).collect();
    assert_eq!(swipe, [true, true, true, false, false]);
    let wait: Vec<Option<i64>> = all.iter().map(|p| p.settings.server_wait).collect();
    assert_eq!(wait, [Some(60), Some(60), Some(60), Some(60), None]);
    let dnd: Vec<bool> = all
        .iter()
        .map(|p| p.settings.breaks_do_not_disturb)
        .collect();
    assert_eq!(dnd, [false, false, false, false, true]);
    assert!(all.iter().all(|p| p.settings.ring_duration.is_none()));
}

#[test]
fn an_open_occurrence_goes_overdue_after_its_priority_s_due_interval() {
    for (p, after) in [
        (Priority::Minimum, DAY),
        (Priority::Low, DAY),
        (Priority::Medium, HOUR),
        (Priority::High, 0),
        (Priority::Maximum, 0),
    ] {
        let mut c = core();
        with_priority(&mut c, "x", T0, p);
        c.tick(T0).unwrap();
        let just_before = T0 + (after - 1).max(0);
        let i = c.inbox(just_before);
        let (due, overdue) = (i.due.len(), i.overdue.len());
        if after == 0 {
            assert_eq!((due, overdue), (0, 1), "{p:?} is overdue at once");
        } else {
            assert_eq!((due, overdue), (1, 0), "{p:?} is still due");
            assert_eq!(c.next_overdue_at(T0), Some(T0 + after));
            let i = c.inbox(T0 + after);
            assert_eq!((i.due.len(), i.overdue.len()), (0, 1), "{p:?} is overdue");
        }
    }
}

#[test]
fn overdue_counts_from_the_scheduled_time_not_from_a_late_firing() {
    let mut c = core();
    with_priority(&mut c, "late", T0, Priority::Medium);
    // The device was asleep for two hours.
    c.tick(T0 + 2 * HOUR).unwrap();
    let i = c.inbox(T0 + 2 * HOUR);
    assert_eq!(titles(&i.overdue), ["late"]);
    assert_eq!(i.overdue[0].scheduled_at, T0);
    assert_eq!(i.overdue[0].fired_at, T0 + 2 * HOUR);
    assert_eq!(i.overdue[0].overdue_at, T0 + HOUR);
}

#[test]
fn defaults_follow_the_priority_until_overridden() {
    let mut c = core();
    let id = with_priority(&mut c, "x", T0, Priority::Low);
    let overdue_at = |c: &Core| c.snapshot().due[0].overdue_at;
    c.tick(T0).unwrap();
    assert_eq!(overdue_at(&c), T0 + DAY);
    // Changing the priority moves the overdue time.
    let edit = |p| EditReminder {
        priority: Some(p),
        ..Default::default()
    };
    c.edit_reminder(&id, edit(Priority::Medium), T0).unwrap();
    assert_eq!(overdue_at(&c), T0 + HOUR);
    c.edit_reminder(&id, edit(Priority::High), T0 + 1).unwrap();
    assert_eq!(overdue_at(&c), T0);
    // An override stays put when the priority changes.
    let over = EditReminder {
        overdue: Some(Some(Delay::After(3 * HOUR))),
        ..Default::default()
    };
    c.edit_reminder(&id, over, T0 + 2).unwrap();
    assert_eq!(overdue_at(&c), T0 + 3 * HOUR);
    c.edit_reminder(&id, edit(Priority::Minimum), T0 + 3)
        .unwrap();
    assert_eq!(overdue_at(&c), T0 + 3 * HOUR);
    // Clearing the override goes back to following the priority.
    let clear = EditReminder {
        overdue: Some(None),
        ..Default::default()
    };
    c.edit_reminder(&id, clear, T0 + 4).unwrap();
    assert_eq!(overdue_at(&c), T0 + DAY);
    let bad = EditReminder {
        overdue: Some(Some(Delay::After(-1))),
        ..Default::default()
    };
    assert!(matches!(
        c.edit_reminder(&id, bad, T0 + 5),
        Err(Error::BadDuration)
    ));
}

#[test]
fn a_new_reminder_is_medium() {
    let mut c = core();
    let id = c.create_reminder("x", T0, T0 - 1).unwrap();
    assert_eq!(c.state().reminders[&id].priority, Priority::Medium);
}

#[test]
fn the_overdue_section_is_highest_priority_first_then_longest_overdue() {
    let mut c = core();
    // All overdue at T0 + 2 days, fired at different times.
    with_priority(&mut c, "low, old", T0, Priority::Low);
    with_priority(&mut c, "high, new", T0 + 3 * HOUR, Priority::High);
    with_priority(&mut c, "high, old", T0 + HOUR, Priority::High);
    with_priority(&mut c, "max", T0 + 5 * HOUR, Priority::Maximum);
    with_priority(&mut c, "low, newer", T0 + HOUR, Priority::Low);
    with_priority(&mut c, "medium", T0 + 2 * HOUR, Priority::Medium);
    let now = T0 + 2 * DAY;
    c.tick(now).unwrap();
    let i = c.inbox(now);
    assert!(i.due.is_empty());
    assert_eq!(
        titles(&i.overdue),
        [
            "max",
            "high, old",
            "high, new",
            "medium",
            "low, old",
            "low, newer"
        ]
    );
}

#[test]
fn the_due_section_holds_only_what_is_not_yet_overdue() {
    let mut c = core();
    with_priority(&mut c, "later", T0 + HOUR, Priority::Low);
    with_priority(&mut c, "now", T0, Priority::High);
    let now = T0 + HOUR;
    c.tick(now).unwrap();
    let i = c.inbox(now);
    assert_eq!(titles(&i.overdue), ["now"]);
    assert_eq!(titles(&i.due), ["later"]);
}

#[test]
fn closing_an_occurrence_takes_it_out_of_overdue() {
    let mut c = core();
    with_priority(&mut c, "x", T0, Priority::High);
    c.tick(T0).unwrap();
    let id = c.inbox(T0).overdue[0].occurrence_id.clone();
    c.complete(&id, T0 + 1).unwrap();
    assert!(c.inbox(T0 + 1).overdue.is_empty());
    assert_eq!(c.next_overdue_at(T0 + 1), None);
}

#[test]
fn an_expiry_delay_misses_the_occurrence_counted_from_the_scheduled_time() {
    let mut c = core();
    let id = c.create_reminder("x", T0, T0 - 1).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            expiry: Some(vec![Delay::After(2 * HOUR)]),
            ..Default::default()
        },
        T0 - 1,
    )
    .unwrap();
    assert_eq!(c.tick(T0).unwrap().len(), 1);
    assert_eq!(c.next_fire_at(), Some(T0 + 2 * HOUR));
    assert!(c.tick(T0 + 2 * HOUR - 1).unwrap().is_empty());
    assert_eq!(c.snapshot().due.len(), 1);
    c.tick(T0 + 2 * HOUR + 30).unwrap();
    assert!(c.snapshot().due.is_empty());
    let o = c.state().occurrences.values().next().unwrap();
    let closing = o.closing.as_ref().unwrap();
    assert_eq!(closing.kind, ClosingKind::Missed);
    assert_eq!(closing.at, T0 + 2 * HOUR, "missed when it expired");
}

#[test]
fn a_late_firing_past_its_expiry_is_missed_at_once_and_does_not_alert() {
    let mut c = core();
    let id = c.create_reminder("x", T0, T0 - 1).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            expiry: Some(vec![Delay::After(HOUR)]),
            ..Default::default()
        },
        T0 - 1,
    )
    .unwrap();
    // Asleep through the expiry.
    let fired = c.tick(T0 + 3 * HOUR).unwrap();
    assert!(
        fired.is_empty(),
        "nobody is alerted about a missed occurrence"
    );
    assert!(c.snapshot().due.is_empty());
    let o = c.state().occurrences.values().next().unwrap();
    assert_eq!(o.scheduled_at, T0);
    assert_eq!(o.fired_at, T0 + 3 * HOUR);
    assert_eq!(o.closing.as_ref().unwrap().kind, ClosingKind::Missed);
    assert_eq!(o.closing.as_ref().unwrap().at, T0 + HOUR);
}

fn stored(n: usize, recorded_at: i64, event: Event) -> StoredEvent {
    StoredEvent {
        list_id: "l".into(),
        seq: Some(n as i64),
        event_id: format!("e{n}"),
        device_id: "1".into(),
        author: "u1".into(),
        recorded_at,
        event,
    }
}

#[test]
fn priority_is_a_setting_of_its_own_merged_by_clock() {
    let hlc = |ms: i64, node: &str| Hlc {
        wall_ms: ms,
        counter: 0,
        node: node.into(),
    };
    let edit = |n: usize, h: Hlc, change: Change| {
        stored(
            n,
            T0,
            Event::ReminderEdited {
                reminder_id: "r".into(),
                hlc: h,
                change,
            },
        )
    };
    let mut s = State::default();
    s.apply(&stored(
        1,
        T0 - 10,
        Event::ReminderCreated {
            reminder_id: "r".into(),
            title: "t".into(),
            fire_at: T0,
        },
    ));
    // Two devices set the priority; the later clock wins whatever the order.
    s.apply(&edit(
        3,
        hlc(T0 * 1000 + 5, "b"),
        Change::Priority(Priority::High),
    ));
    s.apply(&edit(
        2,
        hlc(T0 * 1000 + 1, "a"),
        Change::Priority(Priority::Low),
    ));
    // And another setting is judged on its own.
    s.apply(&edit(
        4,
        hlc(T0 * 1000 + 2, "a"),
        Change::Overdue(Some(Delay::After(60))),
    ));
    let r = &s.reminders["r"];
    assert_eq!(r.priority, Priority::High);
    assert_eq!(r.overdue_override, Some(Delay::After(60)));
    assert_eq!(r.overdue_at(1000, &jiff::tz::TimeZone::UTC), 1060);
    let h = s.history("r", Setting::Priority);
    assert_eq!(h.len(), 2);
    assert!(h[0].current && !h[1].current);
    assert_eq!(h[1].change, Change::Priority(Priority::Low));
}

#[test]
fn the_new_settings_need_a_reader_of_the_new_format() {
    let edit = |change| Event::ReminderEdited {
        reminder_id: "r".into(),
        hlc: Hlc::default(),
        change,
    };
    assert_eq!(edit(Change::Priority(Priority::High)).format(), 3);
    assert_eq!(edit(Change::Overdue(None)).format(), 3);
    assert_eq!(edit(Change::Expiry(vec![Delay::After(5)])).format(), 3);
    assert_eq!(edit(Change::Title("t".into())).format(), 1);
    assert_eq!(hab_core::FORMAT_VERSION, 10);

    let mut c = core();
    c.join("u1", "1").unwrap();
    let id = c.create_reminder("x", T0, 0).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            priority: Some(Priority::High),
            ..Default::default()
        },
        1,
    )
    .unwrap();
    let formats: Vec<u32> = c.unsent().unwrap().iter().map(|o| o.format).collect();
    assert_eq!(formats, vec![1, 3]);
}

#[test]
fn a_priority_survives_reopening_the_store() {
    let dir = std::env::temp_dir().join(format!("hab-prio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("db.sqlite");
    let id;
    {
        let mut c = Core::open(&path).unwrap();
        id = with_priority(&mut c, "x", T0, Priority::Maximum);
    }
    let c = Core::open(&path).unwrap();
    assert_eq!(c.state().reminders[&id].priority, Priority::Maximum);
    let _ = std::fs::remove_dir_all(&dir);
}
