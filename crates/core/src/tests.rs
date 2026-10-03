use super::*;

const T0: Millis = 1_700_000_000_000;

fn core() -> Core {
    Core::open_in_memory("me").unwrap()
}

#[test]
fn a_reminder_does_not_fire_before_its_time() {
    let mut c = core();
    c.create_one_off("Call the plumber", T0 + 1000, T0).unwrap();
    assert!(c.fire_due(T0 + 999).unwrap().is_empty());
    assert!(c.inbox().is_empty());
    assert_eq!(c.next_due(), Some(T0 + 1000));
}

#[test]
fn a_reminder_fires_at_its_time_and_lists_under_due() {
    let mut c = core();
    c.create_one_off("Call the plumber", T0 + 1000, T0).unwrap();
    let opened = c.fire_due(T0 + 1000).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].title, "Call the plumber");
    let inbox = c.inbox();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].section, InboxSection::Due);
    assert_eq!(c.next_due(), None);
}

#[test]
fn firing_twice_opens_one_occurrence() {
    let mut c = core();
    c.create_one_off("x", T0, T0).unwrap();
    assert_eq!(c.fire_due(T0).unwrap().len(), 1);
    assert!(c.fire_due(T0 + 5).unwrap().is_empty());
    assert_eq!(c.inbox().len(), 1);
}

#[test]
fn completing_records_who_and_when_and_finishes_the_one_off() {
    let mut c = core();
    let rid = c.create_one_off("x", T0, T0).unwrap();
    let occ = c.fire_due(T0).unwrap().remove(0);
    c.complete(&occ.id, T0 + 50).unwrap();
    assert!(c.inbox().is_empty());
    let done = c.state().occurrence(&occ.id).unwrap();
    assert_eq!(done.status, OccurrenceStatus::Completed);
    assert_eq!(done.completed_by.as_deref(), Some("me"));
    assert_eq!(done.completed_at, Some(T0 + 50));
    assert!(
        c.state()
            .reminders()
            .find(|r| r.id == rid)
            .unwrap()
            .finished
    );
    assert!(c.fire_due(T0 + 100).unwrap().is_empty());
}

#[test]
fn completing_twice_or_unknown_is_an_error() {
    let mut c = core();
    c.create_one_off("x", T0, T0).unwrap();
    let occ = c.fire_due(T0).unwrap().remove(0);
    c.complete(&occ.id, T0).unwrap();
    assert!(matches!(
        c.complete(&occ.id, T0),
        Err(Error::NoOpenOccurrence(_))
    ));
    assert!(matches!(
        c.complete("nope", T0),
        Err(Error::NoOpenOccurrence(_))
    ));
}

#[test]
fn empty_title_is_rejected() {
    assert!(matches!(
        core().create_one_off("  ", T0, T0),
        Err(Error::EmptyTitle)
    ));
}

#[test]
fn restarting_rebuilds_the_same_state_and_fires_what_passed_while_closed() {
    let path = std::env::temp_dir().join(format!("hab-core-{}.db", uuid::Uuid::new_v4()));
    let p = path.to_str().unwrap();
    {
        let mut c = Core::open(p, "me").unwrap();
        c.create_one_off("while closed", T0 + 1000, T0).unwrap();
        c.create_one_off("already done", T0, T0).unwrap();
        let occ = c.fire_due(T0).unwrap().remove(0);
        c.complete(&occ.id, T0 + 1).unwrap();
    }
    let mut c = Core::open(p, "me").unwrap();
    assert!(c.inbox().is_empty());
    assert_eq!(c.next_due(), Some(T0 + 1000));
    // the app was closed past the time
    let opened = c.fire_due(T0 + 60_000).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].title, "while closed");
    drop(c);
    let c = Core::open(p, "me").unwrap();
    assert_eq!(c.inbox().len(), 1);
    let _ = std::fs::remove_file(path);
}
