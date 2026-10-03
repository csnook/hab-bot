//! End to end: the device list, and removing a device with key rotation.

use hab_client::tls::{Pinned, TlsError};
use hab_client::{
    join, sign_in, suggest_passphrase, JoinRequest, KeyId, KeyKind, KeyStore, Profile,
    SignInRequest, SyncError, Syncer,
};
use hab_core::Core;
use hab_proto::wire::{
    AppendBatch, EventPage, FetchEvents, ListRef, ListRefs, RemoveDevice, Rotation, SealedKeys,
};
use hab_proto::{DeviceKeys, Keys, ListKey};
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
    for _ in 0..400 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn key_id(p: &Profile, kind: KeyKind) -> KeyId {
    KeyId {
        server_fingerprint: p.server_fingerprint.clone(),
        username: p.username.clone(),
        kind,
    }
}

/// A device that is running: its core, keys and sync loop.
struct Device {
    profile: Profile,
    store: KeyStore,
    core: Arc<Mutex<Core>>,
    syncer: Arc<Syncer>,
    wake: Arc<Notify>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Device {
    async fn start(
        profile: Profile,
        store: KeyStore,
        core: Arc<Mutex<Core>>,
        dir: tempfile::TempDir,
        signed_in_as: Option<&str>,
    ) -> Device {
        let wake = Arc::new(Notify::new());
        let syncer = Arc::new(
            Syncer::new(&profile, &store, core.clone(), wake.clone(), || {})
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
        Device {
            profile,
            store,
            core,
            syncer,
            wake,
            stop,
            task,
            _dir: dir,
        }
    }

    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }

    fn id(&self) -> i64 {
        self.profile.device_id
    }

    fn remind(&self, title: &str) {
        self.core
            .lock()
            .unwrap()
            .create_reminder(title, T0 + 50_000, now())
            .unwrap();
        self.wake.notify_one();
    }

    fn sees(&self, title: &str) -> bool {
        self.core
            .lock()
            .unwrap()
            .snapshot()
            .upcoming
            .iter()
            .any(|u| u.title == title)
    }

    async fn device_keys(&self) -> DeviceKeys {
        DeviceKeys::from_bytes(
            &self
                .store
                .get(&key_id(&self.profile, KeyKind::Device))
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }

    async fn account_keys(&self) -> Keys {
        Keys::from_bytes(
            &self
                .store
                .get(&key_id(&self.profile, KeyKind::Account))
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }
}

struct Rig {
    server_dir: tempfile::TempDir,
    server: Server,
    password: String,
    desktop: Device,
}

/// A server with one account whose desktop has joined and is syncing.
async fn rig() -> Rig {
    let server_dir = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let server = Server::start(&config(server_dir.path())).await.unwrap();
    let store = KeyStore::file(app.path());
    let mut core = Core::open_in_memory().unwrap();
    core.create_reminder("Water the plants", T0 + 9000, T0)
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
    let desktop = Device::start(profile, store, Arc::new(Mutex::new(core)), app, None).await;
    desktop.syncer.upload_standalone().await.unwrap();
    desktop.wake.notify_one();
    until("the desktop to connect", || {
        desktop.syncer.status().connected
    })
    .await;
    Rig {
        server_dir,
        server,
        password,
        desktop,
    }
}

impl Rig {
    fn conn(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.server_dir.path().join(db::FILE_NAME)).unwrap()
    }

    fn pinned(&self) -> Pinned {
        Pinned::new(
            &self.server.local_addr().to_string(),
            self.server.fingerprint(),
        )
    }

    /// Sign in another device, as the app does, and start syncing it.
    async fn sign_in(&self, name: &str) -> Device {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::file(dir.path());
        let signed = sign_in(
            SignInRequest {
                address: self.server.local_addr().to_string(),
                fingerprint: self.server.fingerprint().to_string(),
                server_name: "Home server".into(),
                username: "chris".into(),
                password: self.password.clone(),
                device_name: name.into(),
                portable: true,
            },
            &store,
        )
        .await
        .unwrap();
        let profile = signed.profile;
        let mut core = Core::open_in_memory().unwrap();
        core.join(
            &format!("u{}", profile.account_id),
            &profile.device_id.to_string(),
        )
        .unwrap();
        let device =
            Device::start(profile, store, Arc::new(Mutex::new(core)), dir, Some(name)).await;
        until(&format!("{name} to connect"), || {
            device.syncer.status().connected
        })
        .await;
        device
    }

    /// The versions of the personal list's key sealed to a device, and who sealed each.
    fn sealed_to(&self, device: i64) -> Vec<(u32, i64)> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT key_version, sealed_by FROM list_keys WHERE device_id = ?1 ORDER BY key_version")
            .unwrap();
        let rows = stmt
            .query_map([device], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        rows.map(Result::unwrap).collect()
    }

    fn count(&self, sql: &str, id: i64) -> i64 {
        self.conn().query_row(sql, [id], |r| r.get(0)).unwrap()
    }
}

fn status_of<T: std::fmt::Debug>(r: Result<T, TlsError>) -> u16 {
    match r {
        Err(TlsError::Status { status, .. }) => status,
        other => panic!("expected the server to say no, got {other:?}"),
    }
}

#[tokio::test]
async fn settings_lists_the_devices_with_their_names_and_when_they_synced() {
    let rig = rig().await;
    let laptop = rig.sign_in("Laptop").await;
    until("the desktop to hear of the laptop", || {
        rig.desktop
            .core
            .lock()
            .unwrap()
            .device_name(&laptop.id().to_string())
            == Some("Laptop")
    })
    .await;

    let listed = rig.desktop.syncer.devices().await.unwrap();
    assert_eq!(listed.len(), 2);
    let desktop = &listed[0];
    assert_eq!(desktop.id, rig.desktop.id());
    assert_eq!(desktop.name.as_deref(), Some("Desktop"));
    assert!(desktop.this_device && !desktop.portable);
    let other = &listed[1];
    assert_eq!(other.id, laptop.id());
    assert_eq!(other.name.as_deref(), Some("Laptop"));
    assert!(!other.this_device && other.portable);
    for d in &listed {
        let at = d.last_synced.expect("both have synced");
        assert!((now() - at).abs() < 120, "synced just now, not {at}");
    }

    // The other device sees the same list from its side.
    let from_laptop = laptop.syncer.devices().await.unwrap();
    assert_eq!(
        from_laptop
            .iter()
            .map(|d| d.this_device)
            .collect::<Vec<_>>(),
        vec![false, true]
    );

    // The server keeps no name for a device.
    let named: i64 = rig
        .conn()
        .query_row("SELECT COUNT(*) FROM devices WHERE name <> ''", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(named, 0);

    laptop.stop().await;
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn removing_a_device_rotates_the_key_and_locks_it_out() {
    let rig = rig().await;
    let desktop = &rig.desktop;
    let laptop = rig.sign_in("Laptop").await;
    let phone = rig.sign_in("Phone").await;
    until("the phone's sign-in to reach the laptop", || {
        laptop
            .core
            .lock()
            .unwrap()
            .device_name(&phone.id().to_string())
            == Some("Phone")
    })
    .await;

    // The phone makes a reminder before it is removed, and everyone gets it.
    phone.remind("From the phone");
    until("the others to get the phone's reminder", || {
        desktop.sees("From the phone") && laptop.sees("From the phone")
    })
    .await;
    // The laptop is told about the phone's sign-in.
    until("the laptop to hear of the phone", || {
        laptop
            .core
            .lock()
            .unwrap()
            .snapshot()
            .sign_in_notices
            .iter()
            .any(|n| n.device_id == phone.id().to_string())
    })
    .await;

    // What a thief of the phone has: its keys, and the account's keys.
    let phone_keys = phone.device_keys().await;
    let account = phone.account_keys().await;
    let list_id = desktop.core.lock().unwrap().personal_list_id().to_string();
    let old_key = ListKey::personal(&account.personal, &list_id);

    // Remove the phone from the desktop.
    desktop.syncer.remove_device(phone.id()).await.unwrap();

    // The server's row is gone and nothing is sealed to or by the phone.
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM devices WHERE id = ?1", phone.id()),
        0
    );
    assert_eq!(
        rig.count(
            "SELECT COUNT(*) FROM list_keys WHERE device_id = ?1 OR sealed_by = ?1",
            phone.id()
        ),
        0
    );
    // The key was rotated: both remaining devices hold version 1 and the
    // new version 2, sealed by a device that is still on the account.
    for d in [desktop.id(), laptop.id()] {
        let sealed = rig.sealed_to(d);
        assert_eq!(sealed.iter().map(|k| k.0).collect::<Vec<_>>(), vec![1, 2]);
        assert!(sealed.iter().all(|k| k.1 != phone.id()));
    }
    // Version 2 is sealed by the device that removed the phone.
    assert_eq!(rig.sealed_to(laptop.id())[1], (2, desktop.id()));
    let listed = desktop.syncer.devices().await.unwrap();
    assert_eq!(
        listed.iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![desktop.id(), laptop.id()]
    );

    // The removed device cannot fetch, send, or ask for keys.
    let events = FetchEvents {
        list_id: list_id.clone(),
        after: 0,
        limit: 10,
    };
    let fetched: Result<EventPage, _> = rig
        .pinned()
        .signed_post("/api/v1/sync/events", &events, phone.id(), &phone_keys)
        .await;
    assert_eq!(status_of(fetched), 401);
    let keys: Result<SealedKeys, _> = rig
        .pinned()
        .signed_post(
            "/api/v1/lists/keys",
            &ListRef {
                list_id: list_id.clone(),
            },
            phone.id(),
            &phone_keys,
        )
        .await;
    assert_eq!(status_of(keys), 401);
    let lists: Result<ListRefs, _> = rig
        .pinned()
        .signed_post("/api/v1/lists", &(), phone.id(), &phone_keys)
        .await;
    assert_eq!(status_of(lists), 401);
    let append: Result<serde_json::Value, _> = rig
        .pinned()
        .signed_post(
            "/api/v1/sync/append",
            &AppendBatch { envelopes: vec![] },
            phone.id(),
            &phone_keys,
        )
        .await;
    assert_eq!(status_of(append), 401);
    // Its connection is closed and it stays off.
    until("the phone to lose its connection", || {
        !phone.syncer.status().connected
    })
    .await;

    // The desktop makes a reminder, and the laptop reads it under the new key.
    desktop.remind("After the removal");
    until("the laptop to read the new reminder", || {
        laptop.sees("After the removal")
    })
    .await;
    // The phone's notice is gone from the laptop, which was told it was removed.
    until("the laptop to hear of the removal", || {
        laptop
            .core
            .lock()
            .unwrap()
            .snapshot()
            .sign_in_notices
            .is_empty()
    })
    .await;
    // The phone, even with every key it ever had, can't read the new event:
    // fetch it as the desktop and try the old key.
    let page: EventPage = rig
        .pinned()
        .signed_post(
            "/api/v1/sync/events",
            &FetchEvents {
                list_id: list_id.clone(),
                after: 0,
                limit: 100,
            },
            desktop.id(),
            &desktop.device_keys().await,
        )
        .await
        .unwrap();
    let newest = page.events.last().unwrap();
    assert!(
        newest.envelope.open(&old_key).is_err(),
        "the old key opens the new event"
    );
    assert!(
        page.events
            .iter()
            .any(|e| e.envelope.open(&old_key).is_ok()),
        "history is older"
    );
    let secret = std::fs::read(rig.server_dir.path().join(db::FILE_NAME)).unwrap();
    assert!(!secret.windows(17).any(|w| w == b"After the removal"));

    // A device added afterwards reads the whole history: what happened before
    // the rotation, what the removed phone made, and what came after.
    let tablet = rig.sign_in("Tablet").await;
    until("the tablet to read everything", || {
        ["Water the plants", "From the phone", "After the removal"]
            .iter()
            .all(|t| tablet.sees(t))
    })
    .await;
    assert_eq!(
        rig.sealed_to(tablet.id())
            .iter()
            .map(|k| k.0)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    // And what the tablet makes is read by the others.
    tablet.remind("From the tablet");
    until("the others to read the tablet's reminder", || {
        desktop.sees("From the tablet") && laptop.sees("From the tablet")
    })
    .await;
    // The phone still can't read it.
    let page: EventPage = rig
        .pinned()
        .signed_post(
            "/api/v1/sync/events",
            &FetchEvents {
                list_id,
                after: 0,
                limit: 100,
            },
            desktop.id(),
            &desktop.device_keys().await,
        )
        .await
        .unwrap();
    assert!(page.events.last().unwrap().envelope.open(&old_key).is_err());

    phone.stop().await;
    tablet.stop().await;
    laptop.stop().await;
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn a_removal_the_server_cannot_complete_changes_nothing() {
    let rig = rig().await;
    let desktop = &rig.desktop;
    let laptop = rig.sign_in("Laptop").await;
    let phone = rig.sign_in("Phone").await;
    let keys = desktop.device_keys().await;
    let list_id = desktop.core.lock().unwrap().personal_list_id().to_string();

    // A device can't be removed from itself.
    assert!(matches!(
        desktop.syncer.remove_device(desktop.id()).await,
        Err(SyncError::OwnDevice)
    ));
    // Nor one the account doesn't have.
    assert!(matches!(
        desktop.syncer.remove_device(9999).await,
        Err(SyncError::UnknownDevice)
    ));
    // Rotating nothing, or not sealing to every remaining device, is refused.
    let request = |lists: Vec<Rotation>| RemoveDevice {
        device_id: phone.id(),
        lists,
    };
    let none: Result<serde_json::Value, _> = rig
        .pinned()
        .signed_post(
            "/api/v1/devices/remove",
            &request(vec![]),
            desktop.id(),
            &keys,
        )
        .await;
    assert_eq!(status_of(none), 400);
    let empty: Result<serde_json::Value, _> = rig
        .pinned()
        .signed_post(
            "/api/v1/devices/remove",
            &request(vec![Rotation {
                list_id,
                keys: vec![],
            }]),
            desktop.id(),
            &keys,
        )
        .await;
    assert_eq!(status_of(empty), 409);
    // The phone is still on the account, with its key, and still syncing.
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM devices WHERE id = ?1", phone.id()),
        1
    );
    assert_eq!(
        rig.sealed_to(phone.id())
            .iter()
            .map(|k| k.0)
            .collect::<Vec<_>>(),
        vec![1]
    );
    desktop.remind("Still shared");
    until("the phone to read it", || phone.sees("Still shared")).await;
    // The phone can't remove the desktop's... itself.
    assert!(matches!(
        phone.syncer.remove_device(phone.id()).await,
        Err(SyncError::OwnDevice)
    ));
    // But any other device can remove it: here the laptop does.
    laptop.syncer.remove_device(phone.id()).await.unwrap();
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM devices WHERE id = ?1", phone.id()),
        0
    );
    until("the desktop to read under the laptop's new key", || {
        laptop.remind("From the laptop");
        desktop.sees("From the laptop")
    })
    .await;
    // The laptop made version 2, and the desktop was given it.
    assert_eq!(rig.sealed_to(desktop.id())[1], (2, laptop.id()));

    phone.stop().await;
    laptop.stop().await;
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}
