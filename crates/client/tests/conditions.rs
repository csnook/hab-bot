//! End to end, against a real server: one user's two devices share a home
//! location, a sun-event reminder and a reminder with time-based conditions,
//! and evaluate them independently to the same answer (#48, ADR 0012).
//!
//! Every event here is recorded at a manual clock set two hours in the past,
//! as the server refuses events recorded more than 10 minutes ahead of its own
//! clock.

use hab_client::{join, sign_in, suggest_passphrase, JoinRequest, KeyStore, SignInRequest, Syncer};
use hab_core::{Alerter, Command, Condition, Core, SunEvent, SunTrigger};
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

const HOUR: i64 = 3_600;

fn weekdays() -> Condition {
    Condition::Days {
        days: ["MO", "TU", "WE", "TH", "FR"].map(String::from).to_vec(),
    }
}

#[tokio::test]
async fn two_devices_share_the_home_and_evaluate_sun_events_and_conditions_alike() {
    let p = pair().await;
    // Five days before the pair's own clock: the server refuses events
    // recorded ahead of its clock, never behind it.
    let t = p.t - 5 * 24 * HOUR;

    // The desktop sets the home location and makes a sunset reminder with
    // conditions, at the manual clock `t` (two hours ago). Everything after
    // is evaluated at instants the test passes in.
    let rid = with(&p.desktop, |c| {
        c.set_home(51.5074, -0.1278, t).unwrap();
        let list = c.personal_list_id().to_string();
        c.create_repeating_reminder_in(
            &list,
            "Close the chickens in",
            vec![],
            vec![SunTrigger {
                event: SunEvent::Sunset,
                offset_minutes: -30,
            }],
            vec![weekdays(), Condition::Daylight],
            Some("Europe/London"),
            t,
        )
        .unwrap()
    });
    p.meet(&p.desktop, &p.phone).await;

    // The phone has the same home and the same reminder.
    for d in [&p.desktop, &p.phone] {
        let c = d.core.lock().unwrap();
        let home = c.home().unwrap_or_else(|| panic!("{} has no home", d.name));
        assert_eq!(
            (home.name.as_str(), home.radius_metres),
            ("Home", 150),
            "{}",
            d.name
        );
        let v = c.reminder_view(&rid).unwrap();
        assert_eq!(v.conditions.len(), 2, "{}", d.name);
        assert!(!v.needs_home, "{}", d.name);
    }

    // Both predict the same instants, on weekdays only. The reminder was made
    // at `t`, so ask for what comes after it.
    let until = t + 4 * 24 * HOUR;
    let (a, b) = (
        with(&p.desktop, |c| c.expected(t, until)),
        with(&p.phone, |c| c.expected(t, until)),
    );
    let times = |v: &[hab_core::ExpectedItem]| v.iter().map(|e| e.scheduled_at).collect::<Vec<_>>();
    assert!(!a.is_empty());
    assert_eq!(times(&a), times(&b));
    assert_eq!(
        with(&p.desktop, |c| c.next_fire_at()),
        with(&p.phone, |c| c.next_fire_at())
    );

    // At that instant both fire the one occurrence, and both alert.
    let due = a[0].scheduled_at;
    let mut alerts = Vec::new();
    for d in [&p.desktop, &p.phone] {
        let fired = with(d, |c| c.tick(due).unwrap());
        assert_eq!(fired.len(), 1, "{}", d.name);
        assert_eq!(fired[0].occurrence_id, format!("{rid}@{due}"));
        let mut alerter = Alerter::new();
        let pass = with(d, |c| alerter.pass(c, due, false).unwrap());
        alerts.push(
            pass.commands
                .iter()
                .filter(|cmd| matches!(cmd, Command::Show(n) if n.title == "Close the chickens in"))
                .count(),
        );
    }
    assert_eq!(alerts, [1, 1]);
    p.meet(&p.desktop, &p.phone).await;
    for d in [&p.desktop, &p.phone] {
        assert_eq!(
            d.core.lock().unwrap().state().occurrences.len(),
            1,
            "{}",
            d.name
        );
    }

    // The phone moves the home location; the desktop hears, and its
    // predictions move with it.
    let before = with(&p.desktop, |c| c.next_fire_at());
    with(&p.phone, |c| c.set_home(40.7128, -74.0060, t + 60).unwrap());
    p.meet(&p.phone, &p.desktop).await;
    assert_eq!(with(&p.desktop, |c| c.home()).unwrap().latitude, 40.7128);
    let after_move = with(&p.desktop, |c| c.next_fire_at());
    assert_ne!(before, after_move);
    assert_eq!(after_move, with(&p.phone, |c| c.next_fire_at()));

    // And clearing it on the desktop reaches the phone, which then has
    // nothing to fire at.
    with(&p.desktop, |c| c.clear_home(t + 120).unwrap());
    p.meet(&p.desktop, &p.phone).await;
    assert_eq!(with(&p.phone, |c| c.home()), None);
    assert_eq!(with(&p.phone, |c| c.next_fire_at()), None);
    assert!(with(&p.phone, |c| c
        .reminder_view(&rid)
        .unwrap()
        .needs_home));
    p.server.shutdown().await;
}
