//! End to end: the personal list syncing through a real server, encrypted.

use hab_client::tls::{Pinned, TlsError};
use hab_client::{
    join, suggest_passphrase, JoinRequest, KeyId, KeyKind, KeyStore, Profile, SyncError, Syncer,
};
use hab_core::{Core, Payload, FORMAT_VERSION, UPDATE_NOTICE};
use hab_proto::wire::{
    AppendBatch, AppendResults, Envelope, EventPage, FetchEvents, ListRef, SealedKeys,
};
use hab_proto::{open_list_key, DeviceKeys, Keys, ListKey};
use hab_server::{db, Config, Server};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{watch, Notify};

const T0: i64 = 1_000_000;
const SECRET_TITLE: &str = "Zebra plumber 7731";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn config(dir: &Path, listen: &str) -> Config {
    Config {
        data_dir: dir.to_path_buf(),
        listen: listen.parse().unwrap(),
        name: Some("Home server".into()),
        ..Config::default()
    }
}

/// One device: its core, keys and syncer.
struct Device {
    core: Arc<Mutex<Core>>,
    syncer: Arc<Syncer>,
    wake: Arc<Notify>,
    keys: DeviceKeys,
    profile: Profile,
}

async fn syncer_for(profile: &Profile, store: &KeyStore, core: &Arc<Mutex<Core>>) -> Device {
    let wake = Arc::new(Notify::new());
    let syncer = Syncer::new(profile, store, core.clone(), wake.clone(), || {})
        .await
        .unwrap();
    let device_key = store
        .get(&KeyId {
            server_fingerprint: profile.server_fingerprint.clone(),
            username: profile.username.clone(),
            kind: KeyKind::Device,
        })
        .await
        .unwrap()
        .unwrap();
    Device {
        core: core.clone(),
        syncer: Arc::new(syncer),
        wake,
        keys: DeviceKeys::from_bytes(&device_key).unwrap(),
        profile: profile.clone(),
    }
}

struct Rig {
    server_dir: tempfile::TempDir,
    _app: tempfile::TempDir,
    server: Server,
    store: KeyStore,
    account: Keys,
    first: Device,
}

/// A server with one account, whose desktop was used standalone first:
/// a reminder made, fired and completed, and one still to fire.
async fn rig() -> Rig {
    let server_dir = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let server = Server::start(&config(server_dir.path(), "127.0.0.1:0"))
        .await
        .unwrap();
    let store = KeyStore::file(app.path());

    let mut core = Core::open_in_memory().unwrap();
    core.create_reminder(SECRET_TITLE, T0, T0).unwrap();
    let fired = core.tick(T0).unwrap();
    core.complete(&fired[0].occurrence_id, T0 + 5).unwrap();
    core.create_reminder("Water the plants", T0 + 9000, T0 + 6)
        .unwrap();

    let joined = join(
        JoinRequest {
            address: server.local_addr().to_string(),
            fingerprint: server.fingerprint().to_string(),
            server_name: "Home server".into(),
            setup_code: server.setup_code(),
            username: "chris".into(),
            display_name: "Chris".into(),
            password: suggest_passphrase(),
            device_name: "Desktop".into(),
            portable: false,
        },
        &store,
    )
    .await
    .unwrap();
    let profile = joined.profile;
    let account = Keys::from_bytes(
        &store
            .get(&KeyId {
                server_fingerprint: profile.server_fingerprint.clone(),
                username: "chris".into(),
                kind: KeyKind::Account,
            })
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    core.join(
        &format!("u{}", profile.account_id),
        &profile.device_id.to_string(),
    )
    .unwrap();
    let core = Arc::new(Mutex::new(core));
    let first = syncer_for(&profile, &store, &core).await;
    Rig {
        server_dir,
        _app: app,
        server,
        store,
        account,
        first,
    }
}

impl Rig {
    fn conn(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.server_dir.path().join(db::FILE_NAME)).unwrap()
    }

    /// A second device for tests that only need one to exist: vouched for by
    /// the identity key and inserted directly. Signing in for real is tested
    /// in `signin.rs`.
    async fn second_device(&self) -> Device {
        let keys = DeviceKeys::generate();
        let record = keys.record(&self.account.identity, true);
        let id = {
            let conn = self.conn();
            conn.execute(
                "INSERT INTO devices (account_id, name, portable, alg, signing_public,
                     sealing_public, signature, created_at) VALUES (?1, 'Phone', 1, ?2, ?3, ?4, ?5, 0)",
                rusqlite::params![
                    self.first.profile.account_id,
                    record.alg,
                    record.signing_public,
                    record.sealing_public,
                    record.signature
                ],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::file(dir.path());
        let profile = Profile {
            device_id: id,
            device_name: "Phone".into(),
            portable: true,
            ..self.first.profile.clone()
        };
        let key_id = |kind| KeyId {
            server_fingerprint: profile.server_fingerprint.clone(),
            username: "chris".into(),
            kind,
        };
        store
            .put(&key_id(KeyKind::Account), &self.account.to_bytes())
            .await
            .unwrap();
        store
            .put(&key_id(KeyKind::Device), &keys.to_bytes())
            .await
            .unwrap();
        let mut core = Core::open_in_memory().unwrap();
        core.join(&format!("u{}", profile.account_id), &id.to_string())
            .unwrap();
        let core = Arc::new(Mutex::new(core));
        let device = syncer_for(&profile, &store, &core).await;
        // A device added to the account takes the account's personal list.
        let lists = device.syncer.account_lists().await.unwrap();
        assert_eq!(lists.len(), 1);
        core.lock().unwrap().use_personal_list(&lists[0]).unwrap();
        std::mem::forget(dir); // the key file is read once, at construction
        device
    }
}

async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..200 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn titles(core: &Arc<Mutex<Core>>) -> Vec<String> {
    let s = core.lock().unwrap().snapshot();
    let mut t: Vec<String> = s.upcoming.iter().map(|u| u.title.clone()).collect();
    t.extend(s.due.iter().map(|d| d.title.clone()));
    t.sort();
    t
}

fn server_seqs(rig: &Rig) -> Vec<i64> {
    let conn = rig.conn();
    let mut stmt = conn.prepare("SELECT seq FROM events ORDER BY seq").unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[tokio::test]
async fn joining_uploads_the_standalone_history_and_the_server_numbers_it() {
    let rig = rig().await;
    let d = &rig.first;
    // Before joining the upload, nothing has been sent.
    assert!(d.core.lock().unwrap().snapshot().upcoming[0].not_sent);

    let sent = d.syncer.upload_standalone().await.unwrap();
    assert_eq!(sent, 4); // created, opened, completed, created
    assert_eq!(server_seqs(&rig), vec![1, 2, 3, 4]);
    assert!(d.core.lock().unwrap().unsent().unwrap().is_empty());
    let snap = d.core.lock().unwrap().snapshot();
    assert!(snap.upcoming.iter().all(|u| !u.not_sent));
    assert_eq!(snap.update_notice, None);

    // A second run uploads nothing new and numbers nothing again.
    assert_eq!(d.syncer.upload_standalone().await.unwrap(), 0);
    assert_eq!(server_seqs(&rig), vec![1, 2, 3, 4]);
    rig.server.shutdown().await;
}

#[tokio::test]
async fn the_server_database_holds_no_reminder_content() {
    let rig = rig().await;
    let d = &rig.first;
    d.syncer.upload_standalone().await.unwrap();
    d.core
        .lock()
        .unwrap()
        .create_reminder("Another secret: tax return 2291", T0 + 99, T0 + 10)
        .unwrap();
    d.syncer.upload_unsent().await.unwrap();

    // Not in the file, not in any column of any table, whether as text or in
    // the ciphertext: titles, event types, field names, author or times.
    let file = std::fs::read(rig.server_dir.path().join(db::FILE_NAME)).unwrap();
    let list_id = d.core.lock().unwrap().personal_list_id().to_string();
    for needle in [
        SECRET_TITLE,
        "Water the plants",
        "tax return",
        "reminder_created",
        "occurrence_opened",
        "fire_at",
        "title",
        "recorded_at",
        &format!("u{}", d.profile.account_id),
    ] {
        assert!(
            !file.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "the server's file contains {needle:?}"
        );
    }
    // What it does keep per event is the list, device, size and arrival time.
    let conn = rig.conn();
    let (l, dev, size, received, alg, format): (String, i64, i64, i64, String, u32) = conn
        .query_row(
            "SELECT list_id, device_id, size, received_at, alg, format FROM events WHERE seq = 1",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!((l, dev), (list_id, d.profile.device_id));
    assert!(size > 16 && received >= now() - 60);
    assert_eq!(
        (alg.as_str(), format),
        (hab_proto::wire::Algs::EVENT, FORMAT_VERSION)
    );
    // And no table has a place for content.
    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT m.name || '.' || p.name FROM sqlite_master m, pragma_table_info(m.name) p WHERE m.type = 'table'")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for banned in ["title", "body", "text", "plaintext", "content"] {
        assert!(
            !columns.iter().any(|c| c.ends_with(&format!(".{banned}"))),
            "{columns:?}"
        );
    }
    rig.server.shutdown().await;
}

#[tokio::test]
async fn the_list_key_is_stored_sealed_to_each_device_and_only_they_open_it() {
    let rig = rig().await;
    let first = &rig.first;
    first.syncer.upload_standalone().await.unwrap();
    let second = rig.second_device().await;
    // The first device seals the key to the device that has since been added.
    first.syncer.register_personal_list().await.unwrap();

    let list_id = first.core.lock().unwrap().personal_list_id().to_string();
    let expected = ListKey::personal(&rig.account.personal, &list_id);
    let server = Pinned::new(
        &first.profile.server_address,
        &first.profile.server_fingerprint,
    );

    for (me, other) in [(first, &second), (&second, first)] {
        let copies: SealedKeys = server
            .signed_post(
                "/api/v1/lists/keys",
                &ListRef {
                    list_id: list_id.clone(),
                },
                me.profile.device_id,
                &me.keys,
            )
            .await
            .unwrap();
        assert_eq!(copies.keys.len(), 1);
        let sealed = &copies.keys[0];
        assert_eq!(sealed.device_id, me.profile.device_id);
        assert_eq!(sealed.sealed_by, first.profile.device_id);
        let sealer = &first.keys;
        let opened = open_list_key(sealed, &list_id, &me.keys, &sealer.sealing_public()).unwrap();
        assert_eq!(opened.as_bytes(), expected.as_bytes());
        // The other device's private key can't open this device's copy.
        assert!(open_list_key(sealed, &list_id, &other.keys, &sealer.sealing_public()).is_err());
    }
    // The server's table holds ciphertext, never the key.
    let conn = rig.conn();
    let sealed: Vec<Vec<u8>> = conn
        .prepare("SELECT sealed FROM list_keys")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(sealed.len(), 2);
    assert!(sealed
        .iter()
        .all(|s| !s.windows(32).any(|w| w == expected.as_bytes().as_slice())));
    rig.server.shutdown().await;
}

#[tokio::test]
async fn another_device_downloads_the_history_in_bulk() {
    let rig = rig().await;
    rig.first.syncer.upload_standalone().await.unwrap();
    let second = rig.second_device().await;
    assert!(titles(&second.core).is_empty());

    assert_eq!(second.syncer.download().await.expect("download"), 4);
    assert_eq!(titles(&second.core), titles(&rig.first.core));
    // The completion and who did it came across.
    {
        let s = second.core.lock().unwrap();
        assert!(s
            .state()
            .occurrences
            .values()
            .all(|o| o.completed.is_some()));
        let who = s
            .state()
            .occurrences
            .values()
            .next()
            .unwrap()
            .completed
            .clone()
            .unwrap()
            .0;
        assert_eq!(who, format!("u{}", rig.first.profile.account_id));
        assert_eq!(s.cursor().unwrap(), 4);
    }
    // Downloading again finds nothing new.
    assert_eq!(second.syncer.download().await.unwrap(), 0);
    rig.server.shutdown().await;
}

#[tokio::test]
async fn live_events_cross_the_websocket_in_both_directions() {
    let rig = rig().await;
    rig.first.syncer.upload_standalone().await.unwrap();
    let second = rig.second_device().await;

    let (stop_tx, stop_rx) = watch::channel(false);
    let a = {
        let s = rig.first.syncer.clone();
        let rx = stop_rx.clone();
        tokio::spawn(async move { s.run(rx).await })
    };
    let b = {
        let s = second.syncer.clone();
        let rx = stop_rx.clone();
        tokio::spawn(async move { s.run(rx).await })
    };
    until("the history to reach the second device", || {
        titles(&second.core).len() == 1
    })
    .await;
    until("both websockets", || {
        rig.first.syncer.status().connected && second.syncer.status().connected
    })
    .await;

    rig.first
        .core
        .lock()
        .unwrap()
        .create_reminder("From the desktop", T0 + 20_000, T0 + 50)
        .unwrap();
    // Made, but not yet numbered.
    assert!(rig
        .first
        .core
        .lock()
        .unwrap()
        .snapshot()
        .upcoming
        .iter()
        .any(|u| u.title == "From the desktop" && u.not_sent));
    rig.first.wake.notify_one();
    until("the desktop's reminder on the phone", || {
        titles(&second.core).contains(&"From the desktop".to_string())
    })
    .await;
    until("the desktop to see its event numbered", || {
        !rig.first
            .core
            .lock()
            .unwrap()
            .snapshot()
            .upcoming
            .iter()
            .any(|u| u.not_sent)
    })
    .await;

    second
        .core
        .lock()
        .unwrap()
        .create_reminder("From the phone", T0 + 30_000, T0 + 60)
        .unwrap();
    second.wake.notify_one();
    until("the phone's reminder on the desktop", || {
        titles(&rig.first.core).contains(&"From the phone".to_string())
    })
    .await;
    // Both devices agree and the server numbered all six, in one order.
    until("the phone to be numbered", || server_seqs(&rig).len() == 6).await;
    assert_eq!(server_seqs(&rig), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(titles(&second.core), titles(&rig.first.core));

    stop_tx.send(true).unwrap();
    a.await.unwrap();
    b.await.unwrap();
    rig.server.shutdown().await;
}

#[tokio::test]
async fn changes_made_offline_are_not_sent_yet_and_upload_on_reconnecting() {
    let rig = rig().await;
    rig.first.syncer.upload_standalone().await.unwrap();
    let addr = rig.server.local_addr();

    // The server goes away; the user keeps working.
    let Rig {
        server_dir,
        _app,
        server,
        first,
        ..
    } = rig;
    server.shutdown().await;
    {
        let mut core = first.core.lock().unwrap();
        core.create_reminder("Made offline", T0 + 40_000, T0 + 70)
            .unwrap();
        let snap = core.snapshot();
        let item = snap
            .upcoming
            .iter()
            .find(|u| u.title == "Made offline")
            .unwrap();
        assert!(item.not_sent);
        assert!(snap.upcoming.iter().filter(|u| !u.not_sent).count() == 1);
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let task = {
        let s = first.syncer.clone();
        tokio::spawn(async move { s.run(stop_rx).await })
    };
    until("the failure to be noticed", || {
        first.syncer.status().last_error.is_some()
    })
    .await;
    assert!(!first.syncer.status().connected);
    assert!(first
        .core
        .lock()
        .unwrap()
        .snapshot()
        .upcoming
        .iter()
        .any(|u| u.not_sent));

    // The server returns at the same address with the same certificate.
    let server = Server::start(&config(server_dir.path(), &addr.to_string()))
        .await
        .unwrap();
    until("the change to be sent", || {
        first.syncer.status().connected
            && !first
                .core
                .lock()
                .unwrap()
                .snapshot()
                .upcoming
                .iter()
                .any(|u| u.not_sent)
    })
    .await;
    let conn = rusqlite::Connection::open(server_dir.path().join(db::FILE_NAME)).unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 5);

    stop_tx.send(true).unwrap();
    task.await.unwrap();
    server.shutdown().await;
}

/// An event from a newer app, made by a second device with the list's key.
fn newer_event(
    rig: &Rig,
    device: &Device,
    event_id: &str,
    format: u32,
    event: serde_json::Value,
) -> Envelope {
    let list_id = rig
        .first
        .core
        .lock()
        .unwrap()
        .personal_list_id()
        .to_string();
    let payload = Payload {
        author: format!("u{}", rig.first.profile.account_id),
        recorded_at: now(),
        event,
    };
    Envelope::seal(
        &ListKey::personal(&rig.account.personal, &list_id),
        &device.keys,
        device.profile.device_id,
        &list_id,
        event_id,
        format,
        now(),
        &serde_json::to_vec(&payload).unwrap(),
    )
}

async fn append(device: &Device, envelopes: Vec<Envelope>) -> Result<AppendResults, TlsError> {
    Pinned::new(
        &device.profile.server_address,
        &device.profile.server_fingerprint,
    )
    .signed_post(
        "/api/v1/sync/append",
        &AppendBatch { envelopes },
        device.profile.device_id,
        &device.keys,
    )
    .await
}

#[tokio::test]
async fn an_event_in_a_newer_format_is_kept_unapplied_with_a_notice() {
    let rig = rig().await;
    rig.first.syncer.upload_standalone().await.unwrap();
    let second = rig.second_device().await;

    // The server takes any format version.
    let future = newer_event(
        &rig,
        &second,
        "future-1",
        FORMAT_VERSION + 1,
        serde_json::json!({"type": "hologram_created", "shape": "cube"}),
    );
    let known = newer_event(
        &rig,
        &second,
        "known-1",
        FORMAT_VERSION,
        serde_json::json!({"type": "reminder_created", "reminder_id": "r-phone", "title": "Phone reminder", "fire_at": T0 + 77_000}),
    );
    let result = append(&second, vec![future, known]).await.unwrap();
    assert!(result.rejected.is_empty(), "{:?}", result.rejected);
    assert_eq!(
        result.numbered.iter().map(|n| n.seq).collect::<Vec<_>>(),
        vec![5, 6]
    );

    rig.first.syncer.download().await.unwrap();
    let snap = rig.first.core.lock().unwrap().snapshot();
    assert_eq!(snap.update_notice.as_deref(), Some(UPDATE_NOTICE));
    assert_eq!(
        UPDATE_NOTICE,
        "Update the app to see recent changes to this list"
    );
    // The newer event changed nothing; the one after it still applied.
    assert!(titles(&rig.first.core).contains(&"Phone reminder".to_string()));
    assert_eq!(titles(&rig.first.core).len(), 2);
    // It was kept: the held event is still there, and a restart still shows the notice.
    assert_eq!(
        rig.first.core.lock().unwrap().held_events().unwrap(),
        vec!["future-1".to_string()]
    );
    rig.server.shutdown().await;
}

#[tokio::test]
async fn the_server_checks_who_asks_and_who_signed() {
    let rig = rig().await;
    rig.first.syncer.upload_standalone().await.unwrap();
    let second = rig.second_device().await;
    let first = &rig.first;
    let addr = &first.profile.server_address;
    let fp = &first.profile.server_fingerprint;
    let list_id = first.core.lock().unwrap().personal_list_id().to_string();
    let fetch = FetchEvents {
        list_id: list_id.clone(),
        after: 0,
        limit: 10,
    };

    // No signature: refused. The unsigned POST helper sends none.
    let unsigned: Result<EventPage, _> = Pinned::new(addr, fp)
        .post("/api/v1/sync/events", &fetch)
        .await;
    assert!(
        matches!(unsigned, Err(TlsError::Status { status: 401, .. })),
        "{unsigned:?}"
    );
    // Signed by a key that isn't the device's: refused.
    let stranger = DeviceKeys::generate();
    let forged: Result<EventPage, _> = Pinned::new(addr, fp)
        .signed_post(
            "/api/v1/sync/events",
            &fetch,
            first.profile.device_id,
            &stranger,
        )
        .await;
    assert!(matches!(forged, Err(TlsError::Status { status: 401, .. })));
    // Signed properly: works. An unknown list looks the same as someone else's.
    let page: EventPage = Pinned::new(addr, fp)
        .signed_post(
            "/api/v1/sync/events",
            &fetch,
            first.profile.device_id,
            &first.keys,
        )
        .await
        .unwrap();
    assert_eq!(page.events.len(), 4);
    let missing = FetchEvents {
        list_id: "nope".into(),
        ..fetch
    };
    let none: Result<EventPage, _> = Pinned::new(addr, fp)
        .signed_post(
            "/api/v1/sync/events",
            &missing,
            first.profile.device_id,
            &first.keys,
        )
        .await;
    assert!(matches!(none, Err(TlsError::Status { status: 404, .. })));

    // A device can't number an event as another device, or forge its signature.
    let ev = |id: &str| {
        newer_event(
            &rig,
            &second,
            id,
            1,
            serde_json::json!({"type": "reminder_created", "reminder_id": id, "title": "x", "fire_at": 1}),
        )
    };
    let as_other = append(first, vec![ev("as-other")]).await.unwrap();
    assert!(as_other.numbered.is_empty() && as_other.rejected.len() == 1);
    let mut tampered = ev("tampered");
    tampered.ciphertext[0] ^= 1;
    let r = append(&second, vec![tampered]).await.unwrap();
    assert!(r.numbered.is_empty() && r.rejected.len() == 1);
    assert_eq!(server_seqs(&rig).len(), 4);
    rig.server.shutdown().await;
}

#[tokio::test]
async fn duplicate_event_ids_are_refused_and_resends_are_harmless() {
    let rig = rig().await;
    rig.first.syncer.upload_standalone().await.unwrap();
    let second = rig.second_device().await;
    let make = |id: &str, title: &str| {
        newer_event(
            &rig,
            &second,
            id,
            1,
            serde_json::json!({"type": "reminder_created", "reminder_id": id, "title": title, "fire_at": 1}),
        )
    };

    let e = make("dup-1", "one");
    let first_try = append(&second, vec![e.clone()]).await.unwrap();
    assert_eq!(first_try.numbered[0].seq, 5);
    assert!(!first_try.numbered[0].duplicate);
    // The same event again (the answer was lost): same number, nothing added.
    let again = append(&second, vec![e]).await.unwrap();
    assert_eq!(
        (again.numbered[0].seq, again.numbered[0].duplicate),
        (5, true)
    );
    // The same id for a different event: refused.
    let other = append(&second, vec![make("dup-1", "two")]).await.unwrap();
    assert!(other.numbered.is_empty());
    assert!(other.rejected[0].error.contains("already used"));
    assert_eq!(server_seqs(&rig).len(), 5);
    rig.server.shutdown().await;
}

#[tokio::test]
async fn the_server_rejects_clocks_far_ahead_but_not_behind() {
    let rig = rig().await;
    rig.first.syncer.upload_standalone().await.unwrap();
    let second = rig.second_device().await;
    let list_id = rig
        .first
        .core
        .lock()
        .unwrap()
        .personal_list_id()
        .to_string();
    let seal = |id: &str, clock: i64| {
        Envelope::seal(
            &ListKey::personal(&rig.account.personal, &list_id),
            &second.keys,
            second.profile.device_id,
            &list_id,
            id,
            1,
            clock,
            b"{}",
        )
    };
    let ahead = append(&second, vec![seal("future-clock", now() + 3600)])
        .await
        .unwrap();
    assert!(ahead.numbered.is_empty());
    assert!(ahead.rejected[0].error.contains("clock"));
    let slightly = append(&second, vec![seal("small-skew", now() + 30)])
        .await
        .unwrap();
    assert_eq!(slightly.numbered.len(), 1);
    let behind = append(&second, vec![seal("old-clock", now() - 86_400 * 30)])
        .await
        .unwrap();
    assert_eq!(behind.numbered.len(), 1);
    rig.server.shutdown().await;
}

#[tokio::test]
async fn a_device_without_keys_cannot_sync() {
    let rig = rig().await;
    let empty = tempfile::tempdir().unwrap();
    let core = rig.first.core.clone();
    let r = Syncer::new(
        &rig.first.profile,
        &KeyStore::file(empty.path()),
        core,
        Arc::new(Notify::new()),
        || {},
    )
    .await;
    assert!(matches!(r, Err(SyncError::MissingKeys)));
    let _ = &rig.store;
    rig.server.shutdown().await;
}
