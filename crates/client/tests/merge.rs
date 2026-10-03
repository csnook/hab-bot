//! End to end: one user's two devices act without syncing, then meet through a
//! real server, and the merge rules decide (spec: Sync → Which edit wins, and
//! Reminders → Acting on occurrences).

use hab_client::{join, sign_in, suggest_passphrase, JoinRequest, KeyStore, SignInRequest, Syncer};
use hab_core::{Change, ClosingKind, Core, EditReminder, Setting};
use hab_server::{Config, Server};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{watch, Notify};

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..300 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

struct Device {
    core: Arc<Mutex<Core>>,
    syncer: Arc<Syncer>,
}

/// Two signed-in devices, "Desktop" and "Phone", whose sync loops are stopped:
/// they are out of touch until the test uploads and downloads by hand.
struct Pair {
    _server: Server,
    _dirs: Vec<tempfile::TempDir>,
    desktop: Device,
    phone: Device,
}

async fn start_loop(syncer: &Arc<Syncer>) -> (watch::Sender<bool>, tokio::task::JoinHandle<()>) {
    let (stop, rx) = watch::channel(false);
    let s = syncer.clone();
    (stop, tokio::spawn(async move { s.run(rx).await }))
}

async fn pair() -> Pair {
    let server_dir = tempfile::tempdir().unwrap();
    let app1 = tempfile::tempdir().unwrap();
    let app2 = tempfile::tempdir().unwrap();
    let server = Server::start(&Config {
        data_dir: server_dir.path().to_path_buf(),
        listen: "127.0.0.1:0".parse().unwrap(),
        name: Some("Home server".into()),
        ..Config::default()
    })
    .await
    .unwrap();
    let password = suggest_passphrase();
    let store1 = KeyStore::file(app1.path());
    let joined = join(
        JoinRequest {
            address: server.local_addr().to_string(),
            fingerprint: server.fingerprint().to_string(),
            server_name: "Home server".into(),
            setup_code: server.setup_code(),
            username: "chris".into(),
            display_name: "Chris".into(),
            password: password.clone(),
            device_name: "Desktop".into(),
            portable: false,
        },
        &store1,
    )
    .await
    .unwrap();
    let p1 = joined.profile;
    let mut c1 = Core::open_in_memory().unwrap();
    c1.join(&format!("u{}", p1.account_id), &p1.device_id.to_string())
        .unwrap();
    c1.name_device("Desktop", now()).unwrap();
    let core1 = Arc::new(Mutex::new(c1));
    let wake1 = Arc::new(Notify::new());
    let sync1 = Arc::new(
        Syncer::new(&p1, &store1, core1.clone(), wake1, || {})
            .await
            .unwrap(),
    );
    sync1.upload_standalone().await.unwrap();
    let (stop1, task1) = start_loop(&sync1).await;

    // The phone signs in with the password, as in the app.
    let store2 = KeyStore::file(app2.path());
    let p2 = sign_in(
        SignInRequest {
            address: server.local_addr().to_string(),
            fingerprint: server.fingerprint().to_string(),
            server_name: "Home server".into(),
            username: "chris".into(),
            password,
            device_name: "Phone".into(),
            portable: true,
        },
        &store2,
    )
    .await
    .unwrap()
    .profile;
    let mut c2 = Core::open_in_memory().unwrap();
    c2.join(&format!("u{}", p2.account_id), &p2.device_id.to_string())
        .unwrap();
    let core2 = Arc::new(Mutex::new(c2));
    let sync2 = Arc::new(
        Syncer::new(&p2, &store2, core2.clone(), Arc::new(Notify::new()), || {})
            .await
            .unwrap(),
    );
    sync2.adopt_account_list().await.unwrap();
    core2.lock().unwrap().name_device("Phone", now()).unwrap();
    core2
        .lock()
        .unwrap()
        .announce_sign_in("Phone", now())
        .unwrap();
    let (stop2, task2) = start_loop(&sync2).await;

    // Both know each other's names before they part.
    let (c1, c2) = (core1.clone(), core2.clone());
    let (d1, d2) = (p1.device_id.to_string(), p2.device_id.to_string());
    until("the devices to meet", || {
        c1.lock().unwrap().device_name(&d2) == Some("Phone")
            && c2.lock().unwrap().device_name(&d1) == Some("Desktop")
    })
    .await;
    until("the sync loops to be idle", || {
        !c1.lock().unwrap().state().has_unsent()
            && c1.lock().unwrap().unsent().unwrap().is_empty()
            && c2.lock().unwrap().unsent().unwrap().is_empty()
    })
    .await;
    // Now they part: no loops, so nothing syncs until the test says so.
    let _ = stop1.send(true);
    let _ = stop2.send(true);
    let _ = task1.await;
    let _ = task2.await;

    Pair {
        _server: server,
        _dirs: vec![server_dir, app1, app2],
        desktop: Device {
            core: core1,
            syncer: sync1,
        },
        phone: Device {
            core: core2,
            syncer: sync2,
        },
    }
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

    /// A reminder that has fired and reached both devices, still open.
    async fn open_occurrence(&self, title: &str) -> (String, String) {
        let t = now();
        let rid = self
            .desktop
            .core
            .lock()
            .unwrap()
            .create_reminder(title, t - 5, t - 10)
            .unwrap();
        let fired = self.desktop.core.lock().unwrap().tick(t).unwrap();
        self.meet(&self.desktop, &self.phone).await;
        let occ = fired[0].occurrence_id.clone();
        assert!(self
            .phone
            .core
            .lock()
            .unwrap()
            .state()
            .occurrences
            .contains_key(&occ));
        (rid, occ)
    }
}

async fn completion_beats_a_skip(desktop_uploads_first: bool) {
    let pair = pair().await;
    let (_, occ) = pair.open_occurrence("Bins").await;
    // Parted: the desktop completes it, the phone skips it.
    let t = now();
    pair.desktop
        .core
        .lock()
        .unwrap()
        .complete(&occ, t + 1)
        .unwrap();
    pair.phone
        .core
        .lock()
        .unwrap()
        .skip(&occ, Some("later"), t + 2)
        .unwrap();
    assert!(pair.phone.core.lock().unwrap().snapshot().due.is_empty());

    if desktop_uploads_first {
        pair.meet(&pair.desktop, &pair.phone).await;
    } else {
        pair.meet(&pair.phone, &pair.desktop).await;
    }
    for d in [&pair.desktop, &pair.phone] {
        let c = d.core.lock().unwrap();
        let o = &c.state().occurrences[&occ];
        let closing = o.closing.as_ref().unwrap();
        assert_eq!(closing.kind, ClosingKind::Completed);
        assert_eq!(closing.at, t + 1);
        assert!(c.snapshot().due.is_empty());
        assert!(!c.state().has_unsent());
        let notices = c.snapshot().reconciliations;
        assert_eq!(notices.len(), 1);
        assert_eq!(
            notices[0].text,
            "Your Phone skipped \u{201c}Bins\u{201d}. It counts as completed."
        );
    }
}

#[tokio::test]
async fn a_completion_on_one_device_beats_a_skip_on_the_other_and_the_banner_says_so() {
    completion_beats_a_skip(true).await;
}

#[tokio::test]
async fn the_same_holds_when_the_skip_reaches_the_server_first() {
    completion_beats_a_skip(false).await;
}

#[tokio::test]
async fn matching_actions_merge_silently_and_the_first_counts() {
    let pair = pair().await;
    let (_, occ) = pair.open_occurrence("Bins").await;
    let t = now();
    pair.desktop
        .core
        .lock()
        .unwrap()
        .skip(&occ, None, t + 1)
        .unwrap();
    pair.phone
        .core
        .lock()
        .unwrap()
        .skip(&occ, Some("busy"), t + 2)
        .unwrap();
    pair.meet(&pair.phone, &pair.desktop).await;
    for d in [&pair.desktop, &pair.phone] {
        let c = d.core.lock().unwrap();
        let closing = c.state().occurrences[&occ].closing.clone().unwrap();
        // The phone's reached the server first, so it counts.
        assert_eq!((closing.kind, closing.at), (ClosingKind::Skipped, t + 2));
        assert!(c.snapshot().reconciliations.is_empty());
    }
}

#[tokio::test]
async fn closing_beats_snoozing_and_acknowledging_across_devices() {
    let pair = pair().await;
    let (_, occ) = pair.open_occurrence("Bins").await;
    let t = now();
    pair.phone
        .core
        .lock()
        .unwrap()
        .skip(&occ, None, t + 1)
        .unwrap();
    {
        let mut d = pair.desktop.core.lock().unwrap();
        d.snooze(&occ, t + 600, t + 2).unwrap();
        d.acknowledge(&occ, t + 3).unwrap();
    }
    pair.meet(&pair.desktop, &pair.phone).await;
    for d in [&pair.desktop, &pair.phone] {
        let c = d.core.lock().unwrap();
        let o = &c.state().occurrences[&occ];
        assert_eq!(o.closing.as_ref().unwrap().kind, ClosingKind::Skipped);
        assert_eq!((o.snoozed_until, o.acknowledged), (None, false));
        assert!(c.snapshot().reconciliations.is_empty());
    }
}

#[tokio::test]
async fn a_reminder_fired_on_both_devices_while_parted_is_one_occurrence() {
    let pair = pair().await;
    let t = now();
    let rid = pair
        .desktop
        .core
        .lock()
        .unwrap()
        .create_reminder("Bins", t + 60, t)
        .unwrap();
    pair.meet(&pair.desktop, &pair.phone).await;
    // Both fire it, offline, at slightly different times.
    let a = pair.desktop.core.lock().unwrap().tick(t + 60).unwrap();
    let b = pair.phone.core.lock().unwrap().tick(t + 63).unwrap();
    assert_eq!(a[0].occurrence_id, format!("{rid}@{}", t + 60));
    assert_eq!(a[0].occurrence_id, b[0].occurrence_id);
    pair.meet(&pair.phone, &pair.desktop).await;
    for d in [&pair.desktop, &pair.phone] {
        let c = d.core.lock().unwrap();
        assert_eq!(c.state().occurrences.len(), 1);
        assert_eq!(c.snapshot().due.len(), 1);
        // The first to reach the server counts.
        assert_eq!(c.snapshot().due[0].fired_at, t + 63);
    }
}

#[tokio::test]
async fn the_setting_changed_last_wins_per_setting_and_the_loser_can_be_restored() {
    let pair = pair().await;
    let t = now();
    let rid = pair
        .desktop
        .core
        .lock()
        .unwrap()
        .create_reminder("Bins", t + 9000, t)
        .unwrap();
    pair.meet(&pair.desktop, &pair.phone).await;

    // Parted, both retitle it, and each changes something else too.
    pair.desktop
        .core
        .lock()
        .unwrap()
        .edit_reminder(
            &rid,
            EditReminder {
                title: Some("Recycling".into()),
                note: Some("Blue bin".into()),
                ..Default::default()
            },
            t + 10,
        )
        .unwrap();
    pair.phone
        .core
        .lock()
        .unwrap()
        .edit_reminder(
            &rid,
            EditReminder {
                title: Some("Compost".into()),
                fire_at: Some(t + 9500),
                ..Default::default()
            },
            t + 20,
        )
        .unwrap();
    // The phone's reaches the server first, but the desktop's other edits survive.
    pair.meet(&pair.phone, &pair.desktop).await;
    for d in [&pair.desktop, &pair.phone] {
        let c = d.core.lock().unwrap();
        let r = &c.state().reminders[&rid];
        assert_eq!(
            (r.title.as_str(), r.fire_at, r.note.as_str()),
            ("Compost", t + 9500, "Blue bin")
        );
        let h = c.state().history(&rid, Setting::Title);
        assert_eq!(
            h.iter().map(|v| v.change.clone()).collect::<Vec<_>>(),
            [
                Change::Title("Compost".into()),
                Change::Title("Recycling".into()),
                Change::Title("Bins".into())
            ]
        );
    }

    // The desktop restores the title that lost; it wins everywhere.
    let lost = pair
        .desktop
        .core
        .lock()
        .unwrap()
        .state()
        .history(&rid, Setting::Title)[1]
        .event_id
        .clone();
    pair.desktop
        .core
        .lock()
        .unwrap()
        .restore_setting(&rid, Setting::Title, &lost, t + 30)
        .unwrap();
    pair.meet(&pair.desktop, &pair.phone).await;
    for d in [&pair.desktop, &pair.phone] {
        assert_eq!(
            d.core.lock().unwrap().state().reminders[&rid].title,
            "Recycling"
        );
    }
}
