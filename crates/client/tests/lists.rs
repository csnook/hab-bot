//! End to end, against a real server: one user's devices make reminder lists,
//! move reminders between them and delete them, and the merge rules and the
//! keys hold (ADR 0009, ADR 0002, ADR 0008).
//!
//! Every event here is recorded at a manual clock set two hours in the past,
//! as the server refuses events recorded more than 10 minutes ahead of its own
//! clock.

use hab_client::{join, sign_in, suggest_passphrase, JoinRequest, KeyStore, SignInRequest, Syncer};
use hab_core::{Alerter, Command, Core, EditReminder, Priority, Setting};
use hab_server::{db, Config, Server};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::sync::Notify;

/// The manual clock: two hours ago, so that nothing here is ahead of the server.
fn base() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 7_200
}

struct Device {
    name: &'static str,
    id: i64,
    core: Arc<Mutex<Core>>,
    syncer: Arc<Syncer>,
}

/// Two signed-in devices, "Desktop" and "Phone", that sync only when the test
/// says so, against a real server.
struct Pair {
    server: Server,
    dirs: Vec<TempDir>,
    password: String,
    desktop: Device,
    phone: Device,
    t: i64,
}

async fn device_for(
    server: &Server,
    name: &'static str,
    first: bool,
    password: &str,
    t: i64,
) -> (Device, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = KeyStore::file(dir.path());
    let profile = if first {
        join(
            JoinRequest {
                address: server.local_addr().to_string(),
                fingerprint: server.fingerprint().to_string(),
                server_name: "Home server".into(),
                setup_code: server.setup_code(),
                username: "chris".into(),
                display_name: "Chris".into(),
                password: password.to_string(),
                device_name: name.into(),
                portable: false,
            },
            &store,
        )
        .await
        .unwrap()
        .profile
    } else {
        sign_in(
            SignInRequest {
                address: server.local_addr().to_string(),
                fingerprint: server.fingerprint().to_string(),
                server_name: "Home server".into(),
                username: "chris".into(),
                password: password.to_string(),
                device_name: name.into(),
                portable: true,
            },
            &store,
        )
        .await
        .unwrap()
        .profile
    };
    let mut core = Core::open_in_memory().unwrap();
    core.set_device_zone("UTC").unwrap();
    core.join(
        &format!("u{}", profile.account_id),
        &profile.device_id.to_string(),
    )
    .unwrap();
    let core = Arc::new(Mutex::new(core));
    let syncer = Arc::new(
        Syncer::new(
            &profile,
            &store,
            core.clone(),
            Arc::new(Notify::new()),
            || {},
        )
        .await
        .unwrap(),
    );
    if first {
        core.lock().unwrap().name_device(name, t).unwrap();
        syncer.upload_standalone().await.unwrap();
    } else {
        syncer.adopt_account_list().await.unwrap();
        core.lock().unwrap().name_device(name, t).unwrap();
        core.lock().unwrap().announce_sign_in(name, t).unwrap();
    }
    (
        Device {
            name,
            id: profile.device_id,
            core,
            syncer,
        },
        dir,
    )
}

async fn pair() -> Pair {
    let t = base();
    let server_dir = tempfile::tempdir().unwrap();
    let server = Server::start(&Config {
        data_dir: server_dir.path().to_path_buf(),
        listen: "127.0.0.1:0".parse().unwrap(),
        name: Some("Home server".into()),
        ..Config::default()
    })
    .await
    .unwrap();
    let password = suggest_passphrase();
    let (desktop, d1) = device_for(&server, "Desktop", true, &password, t).await;
    let (phone, d2) = device_for(&server, "Phone", false, &password, t).await;
    let pair = Pair {
        server,
        dirs: vec![server_dir, d1, d2],
        password,
        desktop,
        phone,
        t,
    };
    pair.meet(&pair.desktop, &pair.phone).await;
    pair
}

impl Pair {
    /// The devices meet: each sends what it made, in this order, then both
    /// download everything.
    async fn meet(&self, first: &Device, second: &Device) {
        first.syncer.upload_unsent().await.unwrap();
        second.syncer.upload_unsent().await.unwrap();
        self.desktop.syncer.download().await.unwrap();
        self.phone.syncer.download().await.unwrap();
    }

    fn conn(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dirs[0].path().join(db::FILE_NAME)).unwrap()
    }

    /// The key versions sealed to a device for a list.
    fn versions(&self, list: &str, device: i64) -> Vec<u32> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT key_version FROM list_keys
                 WHERE list_id = ?1 AND device_id = ?2 ORDER BY key_version",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![list, device], |r| r.get(0))
            .unwrap();
        rows.map(Result::unwrap).collect()
    }
}

fn with<T>(d: &Device, f: impl FnOnce(&mut Core) -> T) -> T {
    f(&mut d.core.lock().unwrap())
}

fn live_in(c: &Core, id: &str) -> Vec<String> {
    c.lists()
        .into_iter()
        .filter(|l| c.state_of(&l.id).unwrap().reminders.contains_key(id))
        .map(|l| l.id)
        .collect()
}

#[tokio::test]
async fn a_new_list_gets_its_own_stream_and_reaches_the_other_device() {
    let p = pair().await;
    let t = p.t;
    let (home, rid) = with(&p.desktop, |c| {
        let home = c.create_list("Home chores", Some("#e04f5f"), t).unwrap();
        let rid = c
            .create_reminder_in(&home, "Take out the bins", t + 3_600, t)
            .unwrap();
        (home, rid)
    });
    // Sending registers the new list first, as the server refuses events for
    // a list it hasn't been told of.
    p.desktop.syncer.upload_unsent().await.unwrap();
    assert!(p
        .desktop
        .syncer
        .account_lists()
        .await
        .unwrap()
        .contains(&home));
    p.phone.syncer.download().await.unwrap();

    let lists = with(&p.phone, |c| c.lists());
    assert_eq!(lists.len(), 2);
    assert_eq!(lists[1].id, home);
    assert_eq!(lists[1].name.as_deref(), Some("Home chores"));
    assert_eq!(lists[1].colour.as_deref(), Some("#e04f5f"));
    assert_eq!(lists[1].reminders, 1);
    assert_eq!(
        live_in(&p.phone.core.lock().unwrap(), &rid),
        vec![home.clone()]
    );

    // The list's key is sealed to both devices, and its events are its own.
    for d in [&p.desktop, &p.phone] {
        assert_eq!(p.versions(&home, d.id), vec![1], "{}", d.name);
    }
    let events_in = |list: &str| -> i64 {
        p.conn()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE list_id = ?1",
                [list],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(events_in(&home), 3, "named, coloured, one reminder");
    // The server can't read the name, the colour or the reminder.
    let raw = std::fs::read(p.dirs[0].path().join(db::FILE_NAME)).unwrap();
    for secret in [&b"Home chores"[..], b"e04f5f", b"Take out the bins"] {
        assert!(!raw.windows(secret.len()).any(|w| w == secret));
    }

    // Renaming and colouring on the phone arrive on the desktop.
    with(&p.phone, |c| {
        c.rename_list(&home, "Household", t + 10).unwrap();
        c.colour_list(&home, "#3584e4", t + 11).unwrap();
    });
    p.meet(&p.phone, &p.desktop).await;
    let l = with(&p.desktop, |c| c.lists());
    assert_eq!(l[1].name.as_deref(), Some("Household"));
    assert_eq!(l[1].colour.as_deref(), Some("#3584e4"));
}

#[tokio::test]
async fn moving_a_reminder_between_lists_keeps_its_history_on_both_devices() {
    let p = pair().await;
    let t = p.t;
    let (home, rid) = with(&p.desktop, |c| {
        let home = c.create_list("Home", None, t).unwrap();
        let rid = c.create_reminder("Bins", t + 60, t).unwrap();
        c.edit_reminder(
            &rid,
            EditReminder {
                title: Some("Bins (Tuesday)".into()),
                ..Default::default()
            },
            t + 1,
        )
        .unwrap();
        (home, rid)
    });
    let fired = with(&p.desktop, |c| c.tick(t + 60).unwrap());
    let occ = fired[0].occurrence_id.clone();
    p.meet(&p.desktop, &p.phone).await;

    // The phone moves it, with its occurrence open.
    with(&p.phone, |c| c.move_reminder(&rid, &home, t + 100).unwrap());
    p.meet(&p.phone, &p.desktop).await;
    for d in [&p.desktop, &p.phone] {
        let c = d.core.lock().unwrap();
        assert_eq!(live_in(&c, &rid), vec![home.clone()], "{}", d.name);
        let s = c.state_of(&home).unwrap();
        assert!(s.occurrences[&occ].is_open());
        assert_eq!(s.history(&rid, Setting::Title).len(), 2);
        assert!(c.state().occurrences.is_empty());
    }
    // Acting on the open occurrence in its new list reaches the other device.
    with(&p.desktop, |c| c.complete(&occ, t + 120).unwrap());
    p.meet(&p.desktop, &p.phone).await;
    with(&p.phone, |c| {
        assert!(c.state_of(&home).unwrap().occurrences[&occ]
            .completed()
            .is_some());
        assert!(c.snapshot().due.is_empty());
    });
}

#[tokio::test]
async fn a_deletion_beats_what_the_other_device_did_meanwhile_whichever_syncs_first() {
    for desktop_first in [true, false] {
        let p = pair().await;
        let t = p.t;
        let rid = with(&p.desktop, |c| {
            let id = c.create_reminder("Bins", t + 60, t).unwrap();
            c.edit_reminder(
                &id,
                EditReminder {
                    priority: Some(Priority::High),
                    ..Default::default()
                },
                t + 1,
            )
            .unwrap();
            id
        });
        let fired = with(&p.desktop, |c| c.tick(t + 60).unwrap());
        let occ = fired[0].occurrence_id.clone();
        p.meet(&p.desktop, &p.phone).await;
        // The desktop's alarm stands.
        let mut alerter = Alerter::new();
        let shown = with(&p.desktop, |c| alerter.pass(c, t + 61, false).unwrap());
        assert!(shown.commands.iter().any(|c| matches!(c, Command::Show(_))));

        // Parted: the phone completes it and edits the note, the desktop
        // deletes the reminder, keeping its history.
        with(&p.phone, |c| {
            c.complete(&occ, t + 70).unwrap();
            c.edit_reminder(
                &rid,
                EditReminder {
                    note: Some("done by hand".into()),
                    ..Default::default()
                },
                t + 71,
            )
            .unwrap();
        });
        with(&p.desktop, |c| c.delete_reminder(&rid, t + 80).unwrap());
        if desktop_first {
            p.meet(&p.desktop, &p.phone).await;
        } else {
            p.meet(&p.phone, &p.desktop).await;
        }
        for d in [&p.desktop, &p.phone] {
            let mut c = d.core.lock().unwrap();
            assert!(c.state().reminders.is_empty(), "{}", d.name);
            let deleted = c.deleted_reminders();
            assert_eq!(deleted.len(), 1);
            assert_eq!(deleted[0].title, "Bins");
            // Its history is kept, including the completion and the note.
            assert!(c.state().occurrences[&occ].completed().is_some());
            assert_eq!(c.state().history(&rid, Setting::Note).len(), 1);
            // It never fires or alerts again.
            assert!(c.tick(t + 100_000).unwrap().is_empty());
            assert!(c.next_fire_at().is_none());
            assert!(c.snapshot().due.is_empty() && c.snapshot().upcoming.is_empty());
        }
        // The desktop's standing alert is taken down.
        let after = with(&p.desktop, |c| alerter.pass(c, t + 90, false).unwrap());
        assert!(after
            .commands
            .iter()
            .any(|c| matches!(c, Command::Close { occurrence_id } if *occurrence_id == occ)));
        p.server.shutdown().await;
    }
}

#[tokio::test]
async fn deleting_with_history_removes_it_from_the_other_device_and_its_late_actions() {
    let p = pair().await;
    let t = p.t;
    let rid = with(&p.desktop, |c| {
        c.create_reminder("Secret", t + 60, t).unwrap()
    });
    let fired = with(&p.desktop, |c| c.tick(t + 60).unwrap());
    let occ = fired[0].occurrence_id.clone();
    p.meet(&p.desktop, &p.phone).await;
    assert!(with(&p.phone, |c| c.stored_events_of(&rid).unwrap()) > 0);

    with(&p.desktop, |c| c.purge_reminder(&rid, t + 70).unwrap());
    // The phone, not knowing, snoozes and acknowledges it.
    with(&p.phone, |c| {
        c.acknowledge(&occ, t + 71).unwrap();
        c.edit_reminder(
            &rid,
            EditReminder {
                note: Some("late".into()),
                ..Default::default()
            },
            t + 72,
        )
        .unwrap();
    });
    p.meet(&p.desktop, &p.phone).await;
    p.meet(&p.phone, &p.desktop).await;
    for d in [&p.desktop, &p.phone] {
        let c = d.core.lock().unwrap();
        assert!(!c.state().knows(&rid), "{}", d.name);
        assert!(c.state().occurrences.is_empty());
        assert!(c.deleted_reminders().is_empty());
        assert_eq!(c.stored_events_of(&rid).unwrap(), 0, "{}", d.name);
        assert!(c.snapshot().due.is_empty());
    }
}

#[tokio::test]
async fn a_list_made_after_a_device_was_removed_starts_with_a_key_the_removed_device_cannot_derive()
{
    let p = pair().await;
    let t = p.t;
    // A list made before the removal gets the first key, as the removal
    // then rotates every list the account has.
    let before = with(&p.desktop, |c| c.create_list("Before", None, t).unwrap());
    with(&p.desktop, |c| {
        c.create_reminder_in(&before, "x", t + 5_000, t).unwrap()
    });
    p.meet(&p.desktop, &p.phone).await;
    p.desktop.syncer.remove_device(p.phone.id).await.unwrap();
    let personal = with(&p.desktop, |c| c.personal_list_id().to_string());
    assert_eq!(p.versions(&before, p.desktop.id), vec![1, 2]);
    assert_eq!(p.versions(&personal, p.desktop.id), vec![1, 2]);

    // Made afterwards: version 1 would be derivable from the personal key the
    // removed phone still has, so it also gets a random version 2.
    let after = with(&p.desktop, |c| {
        c.create_list("After", None, t + 10).unwrap()
    });
    let rid = with(&p.desktop, |c| {
        c.create_reminder_in(&after, "From after", t + 6_000, t + 10)
            .unwrap()
    });
    p.desktop.syncer.upload_unsent().await.unwrap();
    assert_eq!(p.versions(&after, p.desktop.id), vec![1, 2]);

    // A device that signs in afterwards reads it, under the new key.
    let (tablet, _dir) = device_for(&p.server, "Tablet", false, &p.password, t + 20).await;
    // As the running app does when it hears of a new device, the desktop
    // seals every key to it.
    p.desktop.syncer.register_lists().await.unwrap();
    tablet.syncer.download().await.unwrap();
    with(&tablet, |c| {
        let l = c.lists();
        assert!(l
            .iter()
            .any(|l| l.id == after && l.name.as_deref() == Some("After")));
        assert_eq!(live_in(c, &rid), vec![after.clone()]);
    });
    assert_eq!(p.versions(&after, tablet.id), vec![1, 2]);
    // And what the tablet makes in it is read by the desktop.
    let rid2 = with(&tablet, |c| {
        c.create_reminder_in(&after, "From the tablet", t + 7_000, t + 21)
            .unwrap()
    });
    tablet.syncer.upload_unsent().await.unwrap();
    p.desktop.syncer.download().await.unwrap();
    with(&p.desktop, |c| {
        assert_eq!(live_in(c, &rid2), vec![after.clone()])
    });
}
