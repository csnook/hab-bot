//! End to end, against a real server: one user's two devices and Skip all… on
//! the Inbox's older-quiet row (#54), including a completion on the other
//! device meanwhile.
//!
//! Every event here is recorded at a manual clock set well in the past (40
//! days ago, so a month later is still behind the server's own clock), as the
//! server refuses events recorded more than 10 minutes ahead of it.

use hab_client::{join, sign_in, suggest_passphrase, JoinRequest, KeyStore, SignInRequest, Syncer};
use hab_core::{ClosingKind, Core, EditReminder, Priority, UndoOutcome};
use hab_server::{Config, Server};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::sync::Notify;

/// The manual clock: 40 days ago, so that nothing here is ahead of the server.
fn base() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 40 * 86_400
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

const DAY: i64 = 86_400;

fn old(c: &mut Core, title: &str, priority: Priority, t: i64) -> String {
    let id = c.create_reminder(title, t + 60, t).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            priority: Some(priority),
            ..Default::default()
        },
        t,
    )
    .unwrap();
    c.tick(t + 60).unwrap()[0].occurrence_id.clone()
}

#[tokio::test]
async fn skip_all_on_the_phone_skips_everywhere_but_a_completion_on_the_desktop_wins() {
    let p = pair().await;
    let t = p.t;
    // Four old quiet occurrences and one Medium, all fired on the desktop.
    let (quiet, medium) = with(&p.desktop, |c| {
        let q: Vec<String> = (0..4)
            .map(|i| {
                let pr = if i % 2 == 0 {
                    Priority::Low
                } else {
                    Priority::Minimum
                };
                old(c, &format!("Quiet {i}"), pr, t)
            })
            .collect();
        (q, old(c, "Important", Priority::Medium, t))
    });
    p.meet(&p.desktop, &p.phone).await;

    // A month later on the manual clock: a week and more overdue, and still
    // ten days behind the server's clock.
    let now = t + 30 * DAY;
    for d in [&p.desktop, &p.phone] {
        let inbox = with(d, |c| c.inbox(now));
        assert_eq!(inbox.overdue.len(), 5, "{}", d.name);
        assert_eq!(inbox.folded.len(), 4, "{}", d.name);
        assert!(!inbox.folded.contains(&medium), "{}", d.name);
    }

    // The phone's Skip all, and the desktop's completion of one, made apart.
    let skipped = with(&p.phone, |c| {
        let ids = c.inbox(now).folded;
        c.skip_folded(&ids, now).unwrap()
    });
    assert_eq!(skipped.len(), 4);
    with(&p.desktop, |c| c.complete(&quiet[1], t + 120).unwrap());
    p.meet(&p.phone, &p.desktop).await;

    for d in [&p.desktop, &p.phone] {
        let c = d.core.lock().unwrap();
        for (i, id) in quiet.iter().enumerate() {
            let kind = c.state().occurrences[id].closing.as_ref().unwrap().kind;
            let want = if i == 1 {
                ClosingKind::Completed
            } else {
                ClosingKind::Skipped
            };
            assert_eq!(kind, want, "{} {id}", d.name);
        }
        assert!(c.state().occurrences[&medium].is_open(), "{}", d.name);
    }

    // Undo all from the desktop reopens the skipped ones everywhere.
    let undone = with(&p.desktop, |c| c.undo_all(&skipped, now + 1).unwrap());
    // The completion that beat the skip is not undone.
    assert_eq!(undone.len(), 3);
    assert!(undone.iter().all(|(_, o)| *o == UndoOutcome::Reopened));
    p.meet(&p.desktop, &p.phone).await;
    for d in [&p.desktop, &p.phone] {
        let c = d.core.lock().unwrap();
        assert!(c.state().occurrences[&quiet[0]].is_open(), "{}", d.name);
        assert!(c.state().occurrences[&quiet[2]].is_open(), "{}", d.name);
        assert_eq!(
            c.state().occurrences[&quiet[1]]
                .closing
                .as_ref()
                .unwrap()
                .kind,
            ClosingKind::Completed,
            "{}",
            d.name
        );
    }
    p.server.shutdown().await;
}
