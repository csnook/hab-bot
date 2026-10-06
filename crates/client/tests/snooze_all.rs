//! End to end, against a real server: one user's two devices share a
//! snooze-all and quiet hours, which are personal settings (#50, ADR 0013).
//!
//! Every event here is recorded at a manual clock set two hours in the past,
//! as the server refuses events recorded more than 10 minutes ahead of its own
//! clock.

use hab_client::{join, sign_in, suggest_passphrase, JoinRequest, KeyStore, SignInRequest, Syncer};
use hab_core::{
    Alerter, Command, Core, EditReminder, Priority, QuietHours, Scope, Source, FORMAT_VERSION,
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

const MIN: i64 = 60;
const HOUR: i64 = 3_600;

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

/// A High reminder that fires at `at`, made on the desktop.
fn reminder(c: &mut Core, title: &str, p: Priority, at: i64, t: i64) -> String {
    let id = c.create_reminder(title, at, t).unwrap();
    c.edit_reminder(
        &id,
        EditReminder {
            priority: Some(p),
            ..Default::default()
        },
        t,
    )
    .unwrap();
    id
}

fn shown(commands: &[Command]) -> usize {
    commands
        .iter()
        .filter(|c| matches!(c, Command::Show(_)))
        .count()
}

#[tokio::test]
async fn a_snooze_all_on_the_phone_quiets_the_desktop_and_ending_it_on_the_desktop_alerts_the_phone(
) {
    let p = pair().await;
    let t = p.t;
    let rid = with(&p.desktop, |c| {
        reminder(c, "Bins", Priority::High, t + 10, t)
    });
    let late = with(&p.desktop, |c| {
        reminder(c, "Later", Priority::High, t + 40 * MIN, t)
    });
    p.meet(&p.desktop, &p.phone).await;
    let (mut a_desktop, mut a_phone) = (Alerter::new(), Alerter::new());
    for d in [&p.desktop, &p.phone] {
        assert_eq!(with(d, |c| c.tick(t + 10).unwrap()).len(), 1);
    }
    p.meet(&p.desktop, &p.phone).await;
    let alerts = |d: &Device, a: &mut Alerter, at: i64| {
        with(d, |c| shown(&a.pass(c, at, false).unwrap().commands))
    };
    assert_eq!(alerts(&p.desktop, &mut a_desktop, t + 20), 1);

    // The phone snoozes all, for an hour, Maximum left out.
    let id = with(&p.phone, |c| {
        c.snooze_all(Scope::All, t + HOUR, false, t + MIN).unwrap()
    });
    p.meet(&p.phone, &p.desktop).await;
    for d in [&p.desktop, &p.phone] {
        let c = d.core.lock().unwrap();
        let chips = c.holding(t + 2 * MIN);
        assert_eq!(chips.len(), 1, "{}", d.name);
        assert_eq!(chips[0].until, t + HOUR);
        assert_eq!(chips[0].id.as_deref(), Some(id.as_str()));
    }
    // Neither alerts: not for the open one, nor for the one that fires inside it.
    assert_eq!(alerts(&p.desktop, &mut a_desktop, t + 3 * MIN), 0);
    assert_eq!(alerts(&p.phone, &mut a_phone, t + 3 * MIN), 0);
    for d in [&p.desktop, &p.phone] {
        with(d, |c| c.tick(t + 40 * MIN).unwrap());
    }
    p.meet(&p.desktop, &p.phone).await;
    assert_eq!(alerts(&p.desktop, &mut a_desktop, t + 40 * MIN), 0);
    assert_eq!(alerts(&p.phone, &mut a_phone, t + 40 * MIN), 0);
    // Each affected occurrence records its own snooze, marked as part of it.
    for d in [&p.desktop, &p.phone] {
        for occ in [
            format!("{rid}@{}", t + 10),
            format!("{late}@{}", t + 40 * MIN),
        ] {
            let h = with(d, |c| c.snooze_history(&occ, t + 41 * MIN));
            assert_eq!(h.len(), 1, "{} {occ}", d.name);
            assert_eq!(h[0].via, Some(Source::SnoozeAll));
        }
    }

    // The desktop ends it; the phone alerts for both once it has heard.
    with(&p.desktop, |c| c.end_snooze_all(&id, t + 45 * MIN).unwrap());
    assert_eq!(alerts(&p.phone, &mut a_phone, t + 46 * MIN), 0);
    p.meet(&p.desktop, &p.phone).await;
    assert_eq!(alerts(&p.phone, &mut a_phone, t + 47 * MIN), 2);
    assert_eq!(alerts(&p.desktop, &mut a_desktop, t + 47 * MIN), 2);
    p.server.shutdown().await;
}

#[tokio::test]
async fn quiet_hours_set_on_one_device_hold_alerts_on_the_other() {
    let p = pair().await;
    let t = p.t;
    // Quiet from the hour before now to two hours on, every day, all in UTC.
    let hour_of = |at: i64| at.rem_euclid(86_400) / HOUR;
    let rule = QuietHours {
        days: ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
            .map(String::from)
            .to_vec(),
        from: format!("{:02}:00", (hour_of(t) + 23) % 24),
        to: format!("{:02}:00", (hour_of(t) + 3) % 24),
        scope: Scope::All,
        include_maximum: false,
    };
    with(&p.desktop, |c| {
        c.set_quiet_hours(vec![rule.clone()], t + MIN).unwrap()
    });
    p.meet(&p.desktop, &p.phone).await;
    for d in [&p.desktop, &p.phone] {
        assert_eq!(
            with(d, |c| c.quiet_hours()),
            vec![rule.clone()],
            "{}",
            d.name
        );
    }

    // A reminder fires in the quiet hours on both devices: nobody alerts,
    // Maximum excepted.
    with(&p.desktop, |c| {
        reminder(c, "High", Priority::High, t + 5 * MIN, t + MIN);
        reminder(c, "Max", Priority::Maximum, t + 5 * MIN, t + MIN);
    });
    p.meet(&p.desktop, &p.phone).await;
    for d in [&p.desktop, &p.phone] {
        let mut alerter = Alerter::new();
        with(d, |c| c.tick(t + 5 * MIN).unwrap());
        let n = with(d, |c| {
            shown(&alerter.pass(c, t + 5 * MIN, false).unwrap().commands)
        });
        assert_eq!(n, 1, "{}: only Maximum alerts", d.name);
        let c = d.core.lock().unwrap();
        let holding = c.holding(t + 5 * MIN);
        assert_eq!(holding[0].source, Source::QuietHours);
    }
    assert_eq!(FORMAT_VERSION, 13);
    p.server.shutdown().await;
}
