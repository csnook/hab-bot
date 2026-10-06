//! End to end: two of a user's desktops alert for the same occurrence through a
//! real server, and acting on one applies on the other (spec: Alerts →
//! Alerting on several devices, Reminders → Expiry on late firings).
//!
//! Each device runs what the app runs: a sync loop, and a scheduler that ticks,
//! passes the alerter and sleeps for up to 30 seconds unless woken. The
//! scheduler is woken exactly as in the app: by the syncer's change callback
//! and by the checks coming back. Each device has its own manual clock, so
//! timing is decided by the test, while the waits on the real server are real.

use hab_client::{
    join, sign_in, suggest_passphrase, Checks, JoinRequest, KeyStore, SignInRequest, Syncer,
};
use hab_core::{Alerter, Command, Core, EditReminder, Priority};
use hab_server::{Config, Server};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{watch, Notify};
use tokio::task::JoinHandle;

const HOUR: i64 = 3_600;

/// The manual clocks start five hours in the past, so a test can move them
/// ahead a long way without passing the server's real clock: it refuses
/// events recorded more than 10 minutes in its future.
fn real_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 5 * HOUR
}

/// Waits (in real time) for `ready`, up to `secs`.
async fn until_within(secs: u64, what: &str, mut ready: impl FnMut() -> bool) -> Duration {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(secs) {
        if ready() {
            return started.elapsed();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out after {secs} s waiting for {what}");
}

struct Device {
    id: String,
    core: Arc<Mutex<Core>>,
    syncer: Arc<Syncer>,
    checks: Arc<Checks>,
    /// The scheduler's wake, and the sync loop's.
    changed: Arc<Notify>,
    sync_wake: Arc<Notify>,
    clock: Arc<AtomicI64>,
    commands: Arc<Mutex<Vec<(Instant, Command)>>>,
    sync_loop: Mutex<Option<(watch::Sender<bool>, JoinHandle<()>)>>,
    scheduler: JoinHandle<()>,
    stop_scheduler: watch::Sender<bool>,
}

impl Device {
    fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    /// Time passes on this device alone; its scheduler is woken like a timer.
    fn advance(&self, secs: i64) {
        self.clock.fetch_add(secs, Ordering::SeqCst);
        self.changed.notify_one();
    }

    /// The user acts on this device, as in the app's `changed`.
    fn act<R>(&self, f: impl FnOnce(&mut Core, i64) -> R) -> R {
        let r = f(&mut self.core.lock().unwrap(), self.now());
        self.sync_wake.notify_one();
        self.changed.notify_one();
        r
    }

    fn shows(&self, occ: &str) -> Vec<hab_core::Notification> {
        self.commands
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(_, c)| match c {
                Command::Show(n) if n.occurrence_id == occ => Some(n.clone()),
                _ => None,
            })
            .collect()
    }

    fn closes(&self, occ: &str) -> usize {
        self.closed_at(occ).len()
    }

    fn closed_at(&self, occ: &str) -> Vec<Instant> {
        self.commands
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(at, c)| match c {
                Command::Close { occurrence_id } if occurrence_id == occ => Some(*at),
                _ => None,
            })
            .collect()
    }

    fn alerts_by_me(&self, occ: &str) -> usize {
        self.core
            .lock()
            .unwrap()
            .state()
            .alerts
            .iter()
            .filter(|a| a.occurrence_id == occ && a.device_id == self.id)
            .count()
    }

    fn start_sync(&self) {
        let (stop, rx) = watch::channel(false);
        let s = self.syncer.clone();
        let task = tokio::spawn(async move { s.run(rx).await });
        *self.sync_loop.lock().unwrap() = Some((stop, task));
    }

    async fn stop_sync(&self) {
        let running = self.sync_loop.lock().unwrap().take();
        if let Some((stop, task)) = running {
            let _ = stop.send(true);
            let _ = task.await;
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = self.stop_scheduler.send(true);
        self.scheduler.abort();
        if let Some((stop, task)) = self.sync_loop.lock().unwrap().take() {
            let _ = stop.send(true);
            task.abort();
        }
    }
}

/// The scheduler, as `run_scheduler` in the app does it, sleeping the app's
/// 30 seconds unless woken.
fn spawn_scheduler(
    core: Arc<Mutex<Core>>,
    checks: Arc<Checks>,
    changed: Arc<Notify>,
    sync_wake: Arc<Notify>,
    clock: Arc<AtomicI64>,
    commands: Arc<Mutex<Vec<(Instant, Command)>>>,
) -> (JoinHandle<()>, watch::Sender<bool>) {
    let (stop, mut stopped) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut alerter = Alerter::new();
        loop {
            let now = clock.load(Ordering::SeqCst);
            let (fired, pass) = {
                let mut core = core.lock().unwrap();
                let fired = core.tick(now).unwrap();
                let pass = alerter.pass_with(&mut core, now, false, &*checks).unwrap();
                (fired, pass)
            };
            if !fired.is_empty() || !pass.commands.is_empty() {
                sync_wake.notify_one();
            }
            let at = Instant::now();
            commands
                .lock()
                .unwrap()
                .extend(pass.commands.into_iter().map(|c| (at, c)));
            tokio::select! {
                _ = changed.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                _ = stopped.changed() => return,
            }
        }
    });
    (task, stop)
}

struct Pair {
    server: Option<Server>,
    _dirs: Vec<tempfile::TempDir>,
    /// "Desktop", where reminders are made.
    a: Device,
    /// "Laptop".
    b: Device,
}

#[allow(clippy::too_many_arguments)]
async fn device(
    profile: &hab_client::Profile,
    store: &KeyStore,
    core: Core,
    clock: i64,
    checks: Arc<Checks>,
    run_scheduler: bool,
) -> Device {
    let core = Arc::new(Mutex::new(core));
    let changed = Arc::new(Notify::new());
    let sync_wake = Arc::new(Notify::new());
    let wake = changed.clone();
    checks.on_progress({
        let wake = changed.clone();
        move || wake.notify_one()
    });
    let syncer = Arc::new(
        Syncer::new(profile, store, core.clone(), sync_wake.clone(), move || {
            wake.notify_one()
        })
        .await
        .unwrap()
        .with_checks(checks.clone()),
    );
    let clock = Arc::new(AtomicI64::new(clock));
    let commands = Arc::new(Mutex::new(Vec::new()));
    let (scheduler, stop_scheduler) = if run_scheduler {
        spawn_scheduler(
            core.clone(),
            checks.clone(),
            changed.clone(),
            sync_wake.clone(),
            clock.clone(),
            commands.clone(),
        )
    } else {
        (tokio::spawn(async {}), watch::channel(false).0)
    };
    Device {
        id: profile.device_id.to_string(),
        core,
        syncer,
        checks,
        changed,
        sync_wake,
        clock,
        commands,
        sync_loop: Mutex::new(None),
        scheduler,
        stop_scheduler,
    }
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
    let p1 = join(
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
    .unwrap()
    .profile;
    let t = real_now();
    let mut c1 = Core::open_in_memory().unwrap();
    c1.join(&format!("u{}", p1.account_id), &p1.device_id.to_string())
        .unwrap();
    c1.name_device("Desktop", t).unwrap();
    let a = device(&p1, &store1, c1, t, Arc::new(Checks::new()), false).await;
    a.syncer.upload_standalone().await.unwrap();
    a.start_sync();

    let store2 = KeyStore::file(app2.path());
    let p2 = sign_in(
        SignInRequest {
            address: server.local_addr().to_string(),
            fingerprint: server.fingerprint().to_string(),
            server_name: "Home server".into(),
            username: "chris".into(),
            password,
            device_name: "Laptop".into(),
            portable: false,
        },
        &store2,
    )
    .await
    .unwrap()
    .profile;
    let mut c2 = Core::open_in_memory().unwrap();
    c2.join(&format!("u{}", p2.account_id), &p2.device_id.to_string())
        .unwrap();
    let b = device(&p2, &store2, c2, t, Arc::new(Checks::new()), false).await;
    b.syncer.adopt_account_list().await.unwrap();
    b.core.lock().unwrap().name_device("Laptop", t).unwrap();
    b.core
        .lock()
        .unwrap()
        .announce_sign_in("Laptop", t)
        .unwrap();
    b.start_sync();

    let (c1, c2) = (a.core.clone(), b.core.clone());
    let (d1, d2) = (p1.device_id.to_string(), p2.device_id.to_string());
    until_within(20, "the devices to meet", || {
        c1.lock().unwrap().device_name(&d2) == Some("Laptop")
            && c2.lock().unwrap().device_name(&d1) == Some("Desktop")
            && c1.lock().unwrap().unsent().unwrap().is_empty()
            && c2.lock().unwrap().unsent().unwrap().is_empty()
    })
    .await;
    // Both are connected before schedulers start, so the checks are live.
    until_within(20, "both to be connected", || {
        a.syncer.status().connected && b.syncer.status().connected
    })
    .await;
    Pair {
        server: Some(server),
        _dirs: vec![server_dir, app1, app2],
        a,
        b,
    }
}

impl Pair {
    /// Starts each device's scheduler, which until now had nothing to do.
    fn run_schedulers(&mut self) {
        for d in [&mut self.a, &mut self.b] {
            let (task, stop) = spawn_scheduler(
                d.core.clone(),
                d.checks.clone(),
                d.changed.clone(),
                d.sync_wake.clone(),
                d.clock.clone(),
                d.commands.clone(),
            );
            d.scheduler = task;
            d.stop_scheduler = stop;
        }
    }

    /// A reminder of `priority` made on the Desktop that is due `in_secs`
    /// after both devices' clocks, and has reached the Laptop.
    async fn reminder(&self, title: &str, priority: Priority, in_secs: i64) -> (String, String) {
        let fire_at = self.a.now() + in_secs;
        let rid = self.a.act(|c, now| {
            let id = c.create_reminder(title, fire_at, now).unwrap();
            c.edit_reminder(
                &id,
                EditReminder {
                    priority: Some(priority),
                    ..Default::default()
                },
                now,
            )
            .unwrap();
            id
        });
        let core = self.b.core.clone();
        let r = rid.clone();
        until_within(10, "the Laptop to get the reminder", || {
            core.lock().unwrap().state().reminders.contains_key(&r)
        })
        .await;
        let occ = format!("{rid}@{fire_at}");
        (rid, occ)
    }
}

#[tokio::test]
async fn a_device_that_only_learns_of_the_occurrence_from_the_server_alerts_for_it() {
    let mut pair = pair().await;
    pair.run_schedulers();
    let (_, occ) = pair.reminder("Medicine", Priority::Medium, 600).await;
    // Only the Desktop's clock reaches the time: the Laptop's doesn't, so it
    // can't fire the reminder itself.
    pair.a.advance(600);
    let took = until_within(10, "the Laptop to alert", || !pair.b.shows(&occ).is_empty()).await;
    assert!(took < Duration::from_secs(5), "took {took:?}");
    let shown = pair.b.shows(&occ);
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].body, "Due");
    assert_eq!(pair.b.alerts_by_me(&occ), 1);
    assert!(!pair.a.shows(&occ).is_empty(), "the Desktop alerts too");
    // Each device recorded its own alert.
    until_within(10, "both alerts in the history", || {
        let c = pair.a.core.lock().unwrap();
        let ids: Vec<_> = c
            .state()
            .alerts
            .iter()
            .filter(|a| a.occurrence_id == occ)
            .map(|a| a.device_id.clone())
            .collect();
        ids.contains(&pair.a.id) && ids.contains(&pair.b.id)
    })
    .await;
}

#[tokio::test]
async fn a_device_that_learns_late_alerts_at_the_state_the_occurrence_has_now() {
    let mut pair = pair().await;
    pair.run_schedulers();
    let (_, occ) = pair.reminder("Medicine", Priority::Medium, 600).await;
    // The Laptop is away while the Desktop fires, and when it comes back the
    // occurrence has been overdue for ten minutes: Medium is Insistent then.
    pair.b.stop_sync().await;
    pair.a.advance(600);
    until_within(10, "the Desktop to alert", || {
        !pair.a.shows(&occ).is_empty()
    })
    .await;
    pair.a.advance(HOUR + 600);
    pair.b.advance(600 + HOUR + 600);
    // Nothing of it on the Laptop yet.
    assert!(pair.b.shows(&occ).is_empty());
    pair.b.checks.set_link(hab_client::Link::Connecting);
    pair.b.start_sync();
    until_within(10, "the Laptop to alert", || !pair.b.shows(&occ).is_empty()).await;
    let first = &pair.b.shows(&occ)[0];
    assert_eq!(first.body, "Overdue");
    assert_eq!(first.style, hab_core::AlertStyle::Insistent);
}

#[tokio::test]
async fn completing_skipping_snoozing_and_acknowledging_on_one_device_applies_on_the_other() {
    let mut pair = pair().await;
    pair.run_schedulers();
    let mut occs = Vec::new();
    for title in ["Done", "Skipped", "Snoozed", "Acknowledged"] {
        occs.push(pair.reminder(title, Priority::High, 600).await.1);
    }
    pair.a.advance(600);
    pair.b.advance(600);
    for occ in &occs {
        until_within(10, "both to alert", || {
            !pair.a.shows(occ).is_empty() && !pair.b.shows(occ).is_empty()
        })
        .await;
    }

    pair.b.act(|c, now| c.complete(&occs[0], now).unwrap());
    pair.b.act(|c, now| c.skip(&occs[1], None, now).unwrap());
    pair.b
        .act(|c, now| c.snooze_for_interval(&occs[2], now).unwrap());
    pair.b.act(|c, now| c.acknowledge(&occs[3], now).unwrap());

    until_within(5, "the actions to reach the Desktop", || {
        let c = pair.a.core.lock().unwrap();
        let o = |i: usize| c.state().occurrences[&occs[i]].clone();
        o(0).closing.is_some()
            && o(1).closing.is_some()
            && o(2).snoozed_until.is_some()
            && o(3).acknowledged
    })
    .await;
    // And the Desktop's alerts for all four came down.
    for occ in &occs {
        until_within(5, "the Desktop's alert to come down", || {
            pair.a.closes(occ) == 1
        })
        .await;
    }
    // Acting on the Desktop applies on the Laptop in the same way.
    let (_, occ) = pair.reminder("Later", Priority::High, 600).await;
    pair.a.advance(600);
    pair.b.advance(600);
    until_within(10, "both to alert", || {
        !pair.a.shows(&occ).is_empty() && !pair.b.shows(&occ).is_empty()
    })
    .await;
    pair.a.act(|c, now| c.complete(&occ, now).unwrap());
    until_within(5, "the Laptop to stop alerting", || {
        pair.b.closes(&occ) == 1
    })
    .await;
}

#[tokio::test]
async fn a_ringing_alarm_stops_within_five_seconds_of_acting_on_the_other_device() {
    let mut pair = pair().await;
    pair.run_schedulers();
    for action in ["acknowledge", "complete", "skip", "snooze"] {
        let (_, occ) = pair.reminder(action, Priority::High, 600).await;
        pair.a.advance(600);
        pair.b.advance(600);
        // Both ring as alarms.
        until_within(10, "both to ring", || {
            [&pair.a, &pair.b].iter().all(|d| {
                d.shows(&occ)
                    .iter()
                    .any(|n| n.style == hab_core::AlertStyle::Alarm)
            })
        })
        .await;
        let acted = Instant::now();
        match action {
            "acknowledge" => pair.b.act(|c, now| c.acknowledge(&occ, now).unwrap()),
            "complete" => pair.b.act(|c, now| c.complete(&occ, now).unwrap()),
            "skip" => pair.b.act(|c, now| c.skip(&occ, None, now).unwrap()),
            _ => pair
                .b
                .act(|c, now| c.snooze_for_interval(&occ, now).map(|_| ()).unwrap()),
        }
        // The scheduler sleeps 30 s unless woken: the sync loop wakes it.
        until_within(5, "the Desktop's alarm to be taken down", || {
            pair.a.closes(&occ) > 0
        })
        .await;
        let stopped = pair.a.closed_at(&occ)[0].duration_since(acted);
        assert!(
            stopped < Duration::from_secs(5),
            "{action}: took {stopped:?}"
        );
    }
}

#[tokio::test]
async fn a_late_firing_checks_whether_another_device_closed_the_occurrence_first() {
    let mut pair = pair().await;
    pair.run_schedulers();
    let (_, occ) = pair.reminder("Medicine", Priority::High, 600).await;
    // The Laptop is off while the Desktop fires and the user completes it.
    pair.b.stop_sync().await;
    pair.a.advance(600);
    until_within(10, "the Desktop to ring", || !pair.a.shows(&occ).is_empty()).await;
    pair.a.act(|c, now| c.complete(&occ, now).unwrap());
    until_within(10, "the completion to reach the server", || {
        pair.a.core.lock().unwrap().unsent().unwrap().is_empty()
    })
    .await;
    // The Laptop starts at 10:00 the next morning: the reminder is late, and
    // it hasn't been told. Its first pass fires it and has to check first.
    pair.b.advance(600 + 5);
    pair.b.checks.set_link(hab_client::Link::Connecting);
    pair.b.start_sync();
    until_within(10, "the Laptop to hear of the completion", || {
        pair.b
            .core
            .lock()
            .unwrap()
            .state()
            .occurrences
            .get(&occ)
            .is_some_and(|o| o.closing.is_some())
    })
    .await;
    // Let any alert that was going to come, come.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        pair.b.shows(&occ).is_empty(),
        "the Laptop rang for an occurrence that was already done"
    );
    assert_eq!(pair.b.alerts_by_me(&occ), 0);
}

#[tokio::test]
async fn with_the_server_out_of_reach_a_device_alerts_without_waiting() {
    let mut pair = pair().await;
    pair.run_schedulers();
    let (_, occ) = pair.reminder("Medicine", Priority::Medium, 600).await;
    pair.b.stop_sync().await;
    pair.server.take().unwrap().shutdown().await;
    // The Laptop's own clock reaches the time and its reconnect fails.
    pair.b.checks.set_link(hab_client::Link::Connecting);
    pair.b.start_sync();
    pair.b.advance(600);
    let took = until_within(10, "the Laptop to alert", || !pair.b.shows(&occ).is_empty()).await;
    // Well inside the 60 s wait: it didn't hold back for a server it can't reach.
    assert!(took < Duration::from_secs(8), "took {took:?}");
}
