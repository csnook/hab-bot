//! End to end: a device used standalone signs in to an account that already
//! exists, and its reminders come in as a list named after the device.

use hab_client::{
    join, sign_in, suggest_passphrase, JoinRequest, KeyStore, Profile, SignInRequest, Syncer,
};
use hab_core::{Core, EditReminder};
use hab_server::{db, Config, Server};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{watch, Notify};

const T0: i64 = 1_000_000;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn config(dir: &Path) -> Config {
    Config {
        data_dir: dir.to_path_buf(),
        listen: "127.0.0.1:0".parse().unwrap(),
        name: Some("Home server".into()),
        ..Config::default()
    }
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

/// A running device: core, syncer and its loop.
struct Running {
    syncer: Arc<Syncer>,
    wake: Arc<Notify>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Running {
    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

/// Start a device's sync loop. A device that has just signed in takes the
/// account's personal list and announces itself first, as the app does.
async fn run(
    profile: &Profile,
    store: &KeyStore,
    core: Arc<Mutex<Core>>,
    signed_in_as: Option<&str>,
) -> Running {
    let wake = Arc::new(Notify::new());
    let syncer = Arc::new(
        Syncer::new(profile, store, core.clone(), wake.clone(), || {})
            .await
            .unwrap(),
    );
    if let Some(name) = signed_in_as {
        syncer.adopt_account_list().await.unwrap();
        core.lock().unwrap().announce_sign_in(name, now()).unwrap();
    }
    let (stop, stop_rx) = watch::channel(false);
    let task = {
        let syncer = syncer.clone();
        tokio::spawn(async move { syncer.run(stop_rx).await })
    };
    Running {
        syncer,
        wake,
        stop,
        task,
    }
}

struct Rig {
    _server_dir: tempfile::TempDir,
    server_dir_path: std::path::PathBuf,
    _app: tempfile::TempDir,
    server: Server,
    password: String,
    first_profile: Profile,
    first_store: KeyStore,
    first_core: Arc<Mutex<Core>>,
}

/// A server with one account whose desktop made a few reminders while
/// standalone, joined, and named itself.
async fn rig() -> Rig {
    let server_dir = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let server = Server::start(&config(server_dir.path())).await.unwrap();
    let store = KeyStore::file(app.path());
    let mut core = Core::open_in_memory().unwrap();
    core.create_reminder("Zebra plumber 7731", T0, T0).unwrap();
    let fired = core.tick(T0).unwrap();
    core.complete(&fired[0].occurrence_id, T0 + 5).unwrap();
    core.create_reminder("Water the plants", T0 + 9000, T0 + 6)
        .unwrap();
    let password = suggest_passphrase();
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
        &store,
    )
    .await
    .unwrap();
    let profile = joined.profile;
    core.join(
        &format!("u{}", profile.account_id),
        &profile.device_id.to_string(),
    )
    .unwrap();
    core.name_device(&profile.device_name, now()).unwrap();
    Rig {
        server_dir_path: server_dir.path().to_path_buf(),
        _server_dir: server_dir,
        _app: app,
        server,
        password,
        first_profile: profile,
        first_store: store,
        first_core: Arc::new(Mutex::new(core)),
    }
}

impl Rig {
    fn conn(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.server_dir_path.join(db::FILE_NAME)).unwrap()
    }

    fn request(&self, device_name: &str) -> SignInRequest {
        SignInRequest {
            address: self.server.local_addr().to_string(),
            fingerprint: self.server.fingerprint().to_string(),
            server_name: "Home server".into(),
            username: "chris".into(),
            password: self.password.clone(),
            device_name: device_name.into(),
            portable: true,
        }
    }

    /// Sign in a device with the given core, as the app does: join, the
    /// syncer, the account's list, the announcement, then the loop.
    async fn sign_in_device(
        &self,
        name: &str,
        core: Core,
    ) -> (Profile, Arc<Mutex<Core>>, Running, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::file(dir.path());
        let profile = sign_in(self.request(name), &store).await.unwrap().profile;
        let core = Arc::new(Mutex::new(core));
        core.lock()
            .unwrap()
            .join(
                &format!("u{}", profile.account_id),
                &profile.device_id.to_string(),
            )
            .unwrap();
        let running = run(&profile, &store, core.clone(), Some(name)).await;
        (profile, core, running, dir)
    }
}

const FAR: i64 = 4_000_000_000;

/// A laptop used standalone for a while: one finished reminder, one whose
/// occurrence is open, and one that hasn't fired. Returns the ids.
fn standalone_laptop() -> (Core, [String; 3]) {
    let mut core = Core::open_in_memory().unwrap();
    let done = core.create_reminder("Laptop: bins", T0, T0).unwrap();
    let open = core
        .create_reminder("Laptop: call Sam", T0 + 1, T0)
        .unwrap();
    let fired = core.tick(T0 + 2).unwrap();
    let done_occ = fired.iter().find(|f| f.reminder_id == done).unwrap();
    core.complete(&done_occ.occurrence_id, T0 + 5).unwrap();
    let later = core.create_reminder("Laptop: later", FAR, T0 + 6).unwrap();
    (core, [done, open, later])
}

fn titles(core: &Core, list_id: &str) -> Vec<String> {
    let mut t: Vec<String> = core
        .state_of(list_id)
        .map(|s| s.reminders.values().map(|r| r.title.clone()).collect())
        .unwrap_or_default();
    t.sort();
    t
}

fn imported_list(core: &Core) -> Option<String> {
    core.lists()
        .into_iter()
        .find(|l| l.name.as_deref() == Some("Laptop"))
        .map(|l| l.id)
}

#[tokio::test]
async fn a_standalone_device_signing_in_brings_its_reminders_in_as_a_list_named_after_it() {
    let rig = rig().await;
    let first = run(
        &rig.first_profile,
        &rig.first_store,
        rig.first_core.clone(),
        None,
    )
    .await;
    first.syncer.upload_standalone().await.unwrap();
    first.wake.notify_one();
    until("the desktop to connect", || first.syncer.status().connected).await;
    let account_list = rig
        .first_core
        .lock()
        .unwrap()
        .personal_list_id()
        .to_string();
    let account_titles_before = titles(&rig.first_core.lock().unwrap(), &account_list);
    let account_events_before: i64 = rig
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE list_id = ?1",
            [&account_list],
            |r| r.get(0),
        )
        .unwrap();

    let (laptop, [done, open, later]) = standalone_laptop();
    let standalone_list = laptop.personal_list_id().to_string();
    assert_ne!(standalone_list, account_list);
    // Settings made while standalone: a personal one, and one for this device.
    laptop
        .set_personal_setting("quiet_hours", "22:00-07:00")
        .unwrap();
    laptop
        .set_device_setting("loudest_style", "gentle")
        .unwrap();
    let (p2, core2, second, _dir) = rig.sign_in_device("Laptop", laptop).await;

    // The account's list is its personal list; the standalone one is a list of its own.
    {
        let c = core2.lock().unwrap();
        assert_eq!(c.personal_list_id(), account_list);
        let lists = c.lists();
        assert_eq!(lists.len(), 2);
        assert!(lists[0].personal && lists[0].name.is_none());
        assert_eq!(lists[1].id, standalone_list);
        assert_eq!(lists[1].name.as_deref(), Some("Laptop"));
        // Not merged: the personal list has none of the laptop's reminders.
        assert!(titles(&c, &account_list)
            .iter()
            .all(|t| !t.starts_with("Laptop")));
        assert_eq!(
            titles(&c, &standalone_list),
            ["Laptop: bins", "Laptop: call Sam", "Laptop: later"]
        );
        // Settings: personal ones gave way and the user is told; the device's stay.
        assert_eq!(c.personal_setting("quiet_hours").unwrap(), None);
        assert_eq!(
            c.device_setting("loudest_style").unwrap().as_deref(),
            Some("gentle")
        );
        let notices = c.snapshot().notices;
        assert_eq!(notices.len(), 1);
        assert!(notices[0].text.contains("quiet hours"));
        c.dismiss_notice(&notices[0].id).unwrap();
        assert!(c.snapshot().notices.is_empty());
    }

    // The laptop gets the account's reminders, and the desktop gets the new list.
    until("the laptop to download the account's list", || {
        titles(&core2.lock().unwrap(), &account_list) == account_titles_before
    })
    .await;
    until("the desktop to receive the laptop's list", || {
        let c = rig.first_core.lock().unwrap();
        imported_list(&c).is_some_and(|l| titles(&c, &l).len() == 3)
    })
    .await;
    let desktop_list = imported_list(&rig.first_core.lock().unwrap()).unwrap();
    assert_eq!(desktop_list, standalone_list);
    {
        // With its history: the finished one is closed, the open one is open,
        // under the ids they had, and the author is now the account's user.
        let c = rig.first_core.lock().unwrap();
        let s = c.state_of(&desktop_list).unwrap();
        let occ = |rid: &str| {
            s.occurrences
                .values()
                .find(|o| o.reminder_id == rid)
                .unwrap()
        };
        assert!(!occ(&done).is_open());
        assert_eq!(
            occ(&done).completed().unwrap().0,
            format!("u{}", p2.account_id)
        );
        assert!(occ(&open).is_open());
        assert_eq!(occ(&open).id, format!("{open}@{}", T0 + 1));
        assert!(s.reminders.contains_key(&later));
        // Nothing already on the account was touched.
        assert_eq!(titles(&c, &account_list), account_titles_before);
        // And the imported reminder is in the Inbox like any other.
        assert!(c
            .snapshot()
            .due
            .iter()
            .any(|d| d.title == "Laptop: call Sam"));
    }
    let personal_events_now: i64 = rig
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE list_id = ?1",
            [&account_list],
            |r| r.get(0),
        )
        .unwrap();
    // Only the laptop's sign-in announcement was added to the account's own list.
    assert_eq!(personal_events_now, account_events_before + 1);
    let server_lists: i64 = rig
        .conn()
        .query_row("SELECT COUNT(*) FROM lists", [], |r| r.get(0))
        .unwrap();
    assert_eq!(server_lists, 2);

    // It syncs like any other list, both ways.
    let occurrence = format!("{open}@{}", T0 + 1);
    rig.first_core
        .lock()
        .unwrap()
        .complete(&occurrence, now())
        .unwrap();
    rig.first_core
        .lock()
        .unwrap()
        .edit_reminder(
            &later,
            EditReminder {
                title: Some("Laptop: later, renamed".into()),
                ..EditReminder::default()
            },
            now(),
        )
        .unwrap();
    first.wake.notify_one();
    until("the desktop's changes to reach the laptop", || {
        let c = core2.lock().unwrap();
        let s = c.state_of(&standalone_list).unwrap();
        !s.occurrences[&occurrence].is_open()
            && s.reminders[&later].title == "Laptop: later, renamed"
    })
    .await;
    // A reminder on the laptop that fires there is shared back.
    core2
        .lock()
        .unwrap()
        .edit_reminder(
            &later,
            EditReminder {
                fire_at: Some(T0 + 50),
                ..EditReminder::default()
            },
            now(),
        )
        .unwrap();
    let fired = core2.lock().unwrap().tick(T0 + 60).unwrap();
    assert_eq!(fired.len(), 1);
    second.wake.notify_one();
    until("the laptop's firing to reach the desktop", || {
        rig.first_core
            .lock()
            .unwrap()
            .state_of(&standalone_list)
            .unwrap()
            .occurrences
            .contains_key(&fired[0].occurrence_id)
    })
    .await;
    // Nothing is waiting to be sent on either.
    until("everything to be numbered", || {
        !core2
            .lock()
            .unwrap()
            .snapshot()
            .due
            .iter()
            .any(|d| d.not_sent)
    })
    .await;

    second.stop().await;
    first.stop().await;
}

#[tokio::test]
async fn linking_again_or_restarting_does_not_repeat_the_list_or_its_events() {
    let rig = rig().await;
    let first = run(
        &rig.first_profile,
        &rig.first_store,
        rig.first_core.clone(),
        None,
    )
    .await;
    first.syncer.upload_standalone().await.unwrap();
    let (laptop, _) = standalone_laptop();
    let standalone_list = laptop.personal_list_id().to_string();
    let (p2, core2, second, dir) = rig.sign_in_device("Laptop", laptop).await;
    // The app may run the step again after a failure part way.
    second.syncer.adopt_account_list().await.unwrap();
    until("the standalone history to be uploaded", || {
        let c = core2.lock().unwrap();
        c.unsent().unwrap().is_empty()
    })
    .await;
    second.stop().await;
    let count = |list: &str| -> i64 {
        rig.conn()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE list_id = ?1",
                [list],
                |r| r.get(0),
            )
            .unwrap()
    };
    // Three reminders, two firings, a completion, and the list's name.
    let before = count(&standalone_list);
    assert_eq!(before, 3 + 2 + 1 + 1);
    {
        let mut c = core2.lock().unwrap();
        let account = c.personal_list_id().to_string();
        c.link_account(&account, "Laptop", now()).unwrap();
        assert_eq!(c.lists().len(), 2);
    }
    // The app starts again, resuming rather than signing in.
    let again = run(&p2, &KeyStore::file(dir.path()), core2.clone(), None).await;
    until("the restarted device to connect", || {
        again.syncer.status().connected
    })
    .await;
    again.stop().await;
    assert_eq!(core2.lock().unwrap().lists().len(), 2);
    assert_eq!(count(&standalone_list), before);
    first.stop().await;
}

#[tokio::test]
async fn removing_a_device_rotates_the_imported_list_too_and_a_later_device_reads_it() {
    let rig = rig().await;
    let first = run(
        &rig.first_profile,
        &rig.first_store,
        rig.first_core.clone(),
        None,
    )
    .await;
    first.syncer.upload_standalone().await.unwrap();
    first.wake.notify_one();
    until("the desktop to connect", || first.syncer.status().connected).await;
    let (laptop, [_, open, _]) = standalone_laptop();
    let standalone_list = laptop.personal_list_id().to_string();
    let (p2, _core2, second, _dir2) = rig.sign_in_device("Laptop", laptop).await;
    until("the desktop to receive the laptop's list", || {
        let c = rig.first_core.lock().unwrap();
        imported_list(&c).is_some_and(|l| titles(&c, &l).len() == 3)
    })
    .await;
    second.stop().await;

    // The desktop removes the laptop. Both of the account's lists rotate.
    first.syncer.remove_device(p2.device_id).await.unwrap();
    let versions: i64 = rig
        .conn()
        .query_row(
            "SELECT MIN(v) FROM (SELECT MAX(key_version) AS v FROM list_keys GROUP BY list_id)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(versions, 2);

    // A change made after the rotation, then a third device signing in.
    let occurrence = format!("{open}@{}", T0 + 1);
    rig.first_core
        .lock()
        .unwrap()
        .complete(&occurrence, now())
        .unwrap();
    first.wake.notify_one();
    until("the completion to be numbered", || {
        rig.first_core
            .lock()
            .unwrap()
            .snapshot()
            .due
            .iter()
            .all(|d| !d.not_sent)
    })
    .await;
    let (_p3, core3, third, _dir3) = rig
        .sign_in_device("Phone", Core::open_in_memory().unwrap())
        .await;
    until(
        "the phone to read the imported list, rotation and all",
        || {
            let c = core3.lock().unwrap();
            c.state_of(&standalone_list)
                .and_then(|s| s.occurrences.get(&occurrence))
                .is_some_and(|o| !o.is_open())
        },
    )
    .await;
    {
        let c = core3.lock().unwrap();
        let lists = c.lists();
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[1].name.as_deref(), Some("Laptop"));
    }
    third.stop().await;
    first.stop().await;
}
