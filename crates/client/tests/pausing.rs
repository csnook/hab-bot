//! End to end, against a real server: one user's two devices pause and resume
//! a reminder and a list, and what each skipped, and who is alerted, agrees
//! (ADR 0011).
//!
//! Every event here is recorded at a manual clock set two hours in the past,
//! as the server refuses events recorded more than 10 minutes ahead of its own
//! clock.

use hab_client::{join, sign_in, suggest_passphrase, JoinRequest, KeyStore, SignInRequest, Syncer};
use hab_core::{
    Alerter, ClosingKind, Command, Core, Countdown, CountdownUnit, Pause, PauseCause, Priority,
};
use hab_server::{Config, Server};
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
    core: Arc<Mutex<Core>>,
    syncer: Arc<Syncer>,
}

/// Two signed-in devices, "Desktop" and "Phone", that sync only when the test
/// says so, against a real server.
struct Pair {
    server: Server,
    /// Kept so the directories live as long as the test.
    _dirs: Vec<TempDir>,
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
    (Device { name, core, syncer }, dir)
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
        _dirs: vec![server_dir, d1, d2],
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
}

fn with<T>(d: &Device, f: impl FnOnce(&mut Core) -> T) -> T {
    f(&mut d.core.lock().unwrap())
}

const HALF_HOUR: i64 = 1_800;

fn plants(c: &mut Core, t: i64) -> String {
    let id = c
        .create_countdown_reminder(
            "Plants",
            Countdown {
                amount: 30,
                unit: CountdownUnit::Minutes,
                at: None,
            },
            None,
            Some(t),
            t,
        )
        .unwrap();
    c.edit_reminder(
        &id,
        hab_core::EditReminder {
            priority: Some(Priority::High),
            ..Default::default()
        },
        t,
    )
    .unwrap();
    id
}

#[tokio::test]
async fn a_pause_made_on_the_phone_skips_on_both_devices_and_resuming_early_fires_again() {
    let p = pair().await;
    let t = p.t;
    let rid = with(&p.desktop, |c| plants(c, t));
    p.meet(&p.desktop, &p.phone).await;

    // The phone pauses it for a long while.
    let until = t + 9_000;
    with(&p.phone, |c| {
        c.pause_reminder(&rid, Some(until), t + 100).unwrap()
    });
    p.meet(&p.phone, &p.desktop).await;
    for d in [&p.desktop, &p.phone] {
        let c = d.core.lock().unwrap();
        assert!(c.is_paused(&rid, t + 200), "{}", d.name);
        assert_eq!(
            c.reminder_view(&rid).unwrap().pause,
            Some(Pause {
                from: t + 100,
                until: Some(until)
            })
        );
    }

    // Half an hour later both devices are awake; nothing fires or alerts, and
    // both record the same skip.
    let due = t + HALF_HOUR;
    let mut alerter = Alerter::new();
    for d in [&p.desktop, &p.phone] {
        let fired = with(d, |c| c.tick(due).unwrap());
        assert!(fired.is_empty(), "{}", d.name);
        let pass = with(d, |c| alerter.pass(c, due, false).unwrap());
        assert!(pass.commands.is_empty(), "{}", d.name);
    }
    p.meet(&p.desktop, &p.phone).await;
    let skipped = format!("{rid}@{due}");
    for d in [&p.desktop, &p.phone] {
        let c = d.core.lock().unwrap();
        let v = c.closed_occurrence(&skipped).unwrap();
        assert_eq!(v.kind, ClosingKind::Skipped, "{}", d.name);
        // The history attributes the skip to the pause.
        assert_eq!(
            v.paused,
            Some(PauseCause {
                until: Some(until),
                list: false
            })
        );
        assert_eq!(v.history[0].paused, v.paused);
        assert!(c.snapshot().reconciliations.is_empty(), "{}", d.name);
    }

    // The desktop resumes it early; the phone hears, and the next instance,
    // half an hour after the skip, fires on both as one occurrence.
    with(&p.desktop, |c| c.resume_reminder(&rid, t + 2_500).unwrap());
    p.meet(&p.desktop, &p.phone).await;
    let next = due + HALF_HOUR;
    for d in [&p.desktop, &p.phone] {
        assert!(!with(d, |c| c.is_paused(&rid, next)), "{}", d.name);
        let fired = with(d, |c| c.tick(next).unwrap());
        assert_eq!(fired.len(), 1, "{}", d.name);
        assert_eq!(fired[0].occurrence_id, format!("{rid}@{next}"));
    }
    p.meet(&p.desktop, &p.phone).await;
    for d in [&p.desktop, &p.phone] {
        let c = d.core.lock().unwrap();
        assert_eq!(c.state().occurrences.len(), 2, "{}", d.name);
        assert_eq!(
            c.inbox(next + 1).overdue.len() + c.inbox(next + 1).due.len(),
            1
        );
    }
    p.server.shutdown().await;
}

#[tokio::test]
async fn a_pause_and_a_resume_made_out_of_touch_end_the_same_on_both_devices() {
    for desktop_first in [true, false] {
        for resume_is_later in [true, false] {
            let p = pair().await;
            let t = p.t;
            let rid = with(&p.desktop, |c| plants(c, t));
            with(&p.desktop, |c| {
                c.pause_reminder(&rid, Some(t + 9_000), t + 50).unwrap()
            });
            p.meet(&p.desktop, &p.phone).await;

            // Parted: the desktop resumes, the phone moves the end.
            let (resume_at, move_at) = if resume_is_later {
                (t + 300, t + 200)
            } else {
                (t + 200, t + 300)
            };
            with(&p.desktop, |c| c.resume_reminder(&rid, resume_at).unwrap());
            with(&p.phone, |c| {
                c.pause_reminder(&rid, Some(t + 8_000), move_at).unwrap()
            });
            if desktop_first {
                p.meet(&p.desktop, &p.phone).await;
            } else {
                p.meet(&p.phone, &p.desktop).await;
            }
            let on_desktop = with(&p.desktop, |c| c.state().reminders[&rid].pause);
            let on_phone = with(&p.phone, |c| c.state().reminders[&rid].pause);
            assert_eq!(on_desktop, on_phone);
            assert_eq!(
                on_desktop.is_none(),
                resume_is_later,
                "the later of the two stands (desktop first: {desktop_first})"
            );
            p.server.shutdown().await;
        }
    }
}

#[tokio::test]
async fn pausing_a_list_on_the_phone_takes_down_the_desktops_alarm_and_silences_the_list() {
    let p = pair().await;
    let t = p.t;
    let (home, rid) = with(&p.desktop, |c| {
        let home = c.create_list("Home", None, t).unwrap();
        let id = c.create_reminder_in(&home, "Medicine", t + 60, t).unwrap();
        c.edit_reminder(
            &id,
            hab_core::EditReminder {
                priority: Some(Priority::High),
                ..Default::default()
            },
            t,
        )
        .unwrap();
        (home, id)
    });
    let fired = with(&p.desktop, |c| c.tick(t + 60).unwrap());
    let occ = fired[0].occurrence_id.clone();
    p.meet(&p.desktop, &p.phone).await;
    let mut alerter = Alerter::new();
    let shown = with(&p.desktop, |c| alerter.pass(c, t + 61, false).unwrap());
    assert!(shown.commands.iter().any(|c| matches!(c, Command::Show(_))));

    // The phone pauses the whole list.
    let until = t + 9_000;
    with(&p.phone, |c| {
        c.pause_list(&home, Some(until), t + 100).unwrap()
    });
    p.meet(&p.phone, &p.desktop).await;
    // The desktop's alarm is closed, and the occurrence is skipped by the list.
    let after = with(&p.desktop, |c| alerter.pass(c, t + 101, false).unwrap());
    assert!(after
        .commands
        .iter()
        .any(|c| matches!(c, Command::Close { occurrence_id } if *occurrence_id == occ)));
    for d in [&p.desktop, &p.phone] {
        let c = d.core.lock().unwrap();
        let v = c.closed_occurrence(&occ).unwrap();
        assert_eq!(
            v.paused,
            Some(PauseCause {
                until: Some(until),
                list: true
            }),
            "{}",
            d.name
        );
        assert_eq!(
            c.lists().iter().find(|l| l.id == home).unwrap().pause,
            Some(Pause {
                from: t + 100,
                until: Some(until)
            })
        );
        assert!(c.is_paused(&rid, t + 200));
        assert!(c.inbox(t + 200).overdue.is_empty() && c.inbox(t + 200).due.is_empty());
    }
    // The list is resumed from the desktop and the phone hears.
    with(&p.desktop, |c| c.resume_list(&home, t + 300).unwrap());
    p.meet(&p.desktop, &p.phone).await;
    assert!(!with(&p.phone, |c| c.is_paused(&rid, t + 400)));
    p.server.shutdown().await;
}
