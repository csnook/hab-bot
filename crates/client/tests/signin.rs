//! End to end: signing in on a second device with the password.

use hab_client::tls::{Pinned, TlsError};
use hab_client::{
    join, sign_in, suggest_passphrase, JoinRequest, KeyId, KeyKind, KeyStore, Profile, SignInError,
    SignInRequest, Syncer,
};
use hab_core::Core;
use hab_proto::opaque_ke::{
    ClientLogin, ClientLoginFinishParameters, CredentialResponse, Identifiers,
};
use hab_proto::wire::{
    DeviceList, LoginDevice, LoginFinish, LoginFinished, LoginStart, LoginStarted,
};
use hab_proto::{argon2, verify_device, DeviceKeys, Keys, ListKey, Suite};
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

fn key_id(p: &Profile, kind: KeyKind) -> KeyId {
    KeyId {
        server_fingerprint: p.server_fingerprint.clone(),
        username: p.username.clone(),
        kind,
    }
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

    fn request(&self, device_name: &str, password: &str) -> SignInRequest {
        SignInRequest {
            address: self.server.local_addr().to_string(),
            fingerprint: self.server.fingerprint().to_string(),
            server_name: "Home server".into(),
            username: "chris".into(),
            password: password.into(),
            device_name: device_name.into(),
            portable: true,
        }
    }

    fn pinned(&self) -> Pinned {
        Pinned::new(
            &self.server.local_addr().to_string(),
            self.server.fingerprint(),
        )
    }

    fn device_count(&self) -> i64 {
        self.conn()
            .query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))
            .unwrap()
    }
}

#[tokio::test]
async fn a_second_device_signs_in_with_the_password_and_everything_follows() {
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
    // The first device's loop is up and following the WebSocket.
    until("the first device to connect", || {
        first.syncer.status().connected
    })
    .await;

    // Sign in on a second device: its own folder, as another login on the
    // same computer would have.
    let second_app = tempfile::tempdir().unwrap();
    let second_store = KeyStore::file(second_app.path());
    let signed = sign_in(rig.request("Laptop", &rig.password), &second_store)
        .await
        .unwrap();
    let p2 = signed.profile;
    assert_eq!(p2.username, "chris");
    assert_eq!(p2.display_name, "Chris");
    assert!(p2.admin);
    assert_eq!(p2.account_id, rig.first_profile.account_id);
    assert_ne!(p2.device_id, rig.first_profile.device_id);
    assert_eq!(p2.server_fingerprint, rig.server.fingerprint());
    assert_eq!(rig.device_count(), 2);

    // It unlocked the account's keys, and made a device key of its own.
    let account1 = Keys::from_bytes(
        &rig.first_store
            .get(&key_id(&rig.first_profile, KeyKind::Account))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let account2 = Keys::from_bytes(
        &second_store
            .get(&key_id(&p2, KeyKind::Account))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(account2.personal, account1.personal);
    assert_eq!(account2.identity_public(), account1.identity_public());
    let device2 = DeviceKeys::from_bytes(
        &second_store
            .get(&key_id(&p2, KeyKind::Device))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let first_device = DeviceKeys::from_bytes(
        &rig.first_store
            .get(&key_id(&rig.first_profile, KeyKind::Device))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_ne!(device2.sealing_public(), first_device.sealing_public());

    // The server holds a device the identity key signed, and nothing of its name.
    let listed: DeviceList = rig
        .pinned()
        .signed_post("/api/v1/devices", &(), p2.device_id, &device2)
        .await
        .unwrap();
    assert_eq!(listed.devices.len(), 2);
    let mine = listed
        .devices
        .iter()
        .find(|d| d.id == p2.device_id)
        .unwrap();
    verify_device(&account1.identity_public(), &mine.record).unwrap();
    assert_eq!(
        mine.record.sealing_public,
        device2.sealing_public().to_vec()
    );
    let names: i64 = rig
        .conn()
        .query_row("SELECT COUNT(*) FROM devices WHERE name <> ''", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(names, 0, "the server's table holds device names");

    // The new device takes the account's personal list, says who it is, and syncs.
    let core2 = Arc::new(Mutex::new(Core::open_in_memory().unwrap()));
    core2
        .lock()
        .unwrap()
        .join(&format!("u{}", p2.account_id), &p2.device_id.to_string())
        .unwrap();
    let second = run(&p2, &second_store, core2.clone(), Some("Laptop")).await;
    let list_id = rig
        .first_core
        .lock()
        .unwrap()
        .personal_list_id()
        .to_string();
    assert_eq!(core2.lock().unwrap().personal_list_id(), list_id);

    // It downloads everything, then follows the WebSocket.
    until("the new device to download", || {
        let s = core2.lock().unwrap().snapshot();
        s.upcoming.len() == 1 && s.upcoming[0].title == "Water the plants"
    })
    .await;
    until("the new device to connect", || {
        second.syncer.status().connected
    })
    .await;
    rig.first_core
        .lock()
        .unwrap()
        .create_reminder("Live from the desktop", T0 + 10_000, now())
        .unwrap();
    first.wake.notify_one();
    until("a live event to reach the new device", || {
        core2
            .lock()
            .unwrap()
            .snapshot()
            .upcoming
            .iter()
            .any(|u| u.title == "Live from the desktop")
    })
    .await;

    // The list key is sealed to the new device, and opens there. The first
    // device seals it too once it hears of the sign-in.
    let expected = ListKey::personal(&account1.personal, &list_id);
    let opened = second.syncer.sealed_list_key().await.unwrap();
    assert_eq!(opened.as_bytes(), expected.as_bytes());
    let first_id = rig.first_profile.device_id;
    until("the first device to seal the key to the new one", || {
        let sealed_by: i64 = rig
            .conn()
            .query_row(
                "SELECT sealed_by FROM list_keys WHERE device_id = ?1",
                [p2.device_id],
                |r| r.get(0),
            )
            .unwrap();
        sealed_by == first_id
    })
    .await;
    let again = second.syncer.sealed_list_key().await.unwrap();
    assert_eq!(again.as_bytes(), expected.as_bytes());

    // The first device is told, in its own words for the new one's name.
    until("the first device to hear of the sign-in", || {
        !rig.first_core
            .lock()
            .unwrap()
            .snapshot()
            .sign_in_notices
            .is_empty()
    })
    .await;
    let notices = rig.first_core.lock().unwrap().snapshot().sign_in_notices;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].device_name, "Laptop");
    assert_eq!(notices[0].device_id, p2.device_id.to_string());
    assert!((now() - notices[0].at).abs() < 60, "just now");
    {
        let c = rig.first_core.lock().unwrap();
        assert_eq!(c.device_name(&p2.device_id.to_string()), Some("Laptop"));
        assert_eq!(c.device_name(&first_id.to_string()), Some("Desktop"));
    }
    // The new device doesn't announce itself to itself, nor older sign-ins.
    assert!(core2.lock().unwrap().snapshot().sign_in_notices.is_empty());
    // Dismissed, it stays gone.
    rig.first_core
        .lock()
        .unwrap()
        .dismiss_notice(&notices[0].id)
        .unwrap();
    assert!(rig
        .first_core
        .lock()
        .unwrap()
        .snapshot()
        .sign_in_notices
        .is_empty());

    // The server never saw a device's name, nor the password.
    let file = std::fs::read(rig.server_dir_path.join(db::FILE_NAME)).unwrap();
    for needle in [
        "Laptop",
        "Desktop",
        "device_signed_in",
        "device_named",
        &rig.password,
    ] {
        assert!(
            !file.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "the server's file contains {needle:?}"
        );
    }

    first.stop().await;
    second.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn a_wrong_password_or_username_signs_in_nothing_and_looks_the_same() {
    let rig = rig().await;
    let app = tempfile::tempdir().unwrap();
    let store = KeyStore::file(app.path());

    let wrong = sign_in(rig.request("Laptop", "not the password at all"), &store).await;
    assert!(matches!(wrong, Err(SignInError::WrongLogin)), "{wrong:?}");
    let mut stranger = rig.request("Laptop", &rig.password);
    stranger.username = "nobody".into();
    let unknown = sign_in(stranger, &store).await;
    assert!(
        matches!(unknown, Err(SignInError::WrongLogin)),
        "{unknown:?}"
    );
    assert_eq!(
        wrong.err().unwrap().to_string(),
        unknown.err().unwrap().to_string()
    );

    assert_eq!(rig.device_count(), 1);
    for kind in [KeyKind::Account, KeyKind::Device] {
        let id = KeyId {
            server_fingerprint: rig.first_profile.server_fingerprint.clone(),
            username: "chris".into(),
            kind,
        };
        assert!(store.get(&id).await.unwrap().is_none());
    }
    rig.server.shutdown().await;
}

#[tokio::test]
async fn the_fingerprint_is_pinned_when_signing_in() {
    let rig = rig().await;
    let app = tempfile::tempdir().unwrap();
    let store = KeyStore::file(app.path());
    let mut req = rig.request("Laptop", &rig.password);
    req.fingerprint = "00:11".repeat(16);
    let result = sign_in(req, &store).await;
    assert!(
        matches!(result, Err(SignInError::Server(TlsError::WrongCertificate))),
        "{result:?}"
    );
    assert_eq!(rig.device_count(), 1);
    rig.server.shutdown().await;
}

/// The three calls by hand, to try what a sign-in client never sends.
struct ByHand {
    login_id: String,
    finished: Option<LoginFinished>,
}

async fn start_by_hand(rig: &Rig, password: &str) -> (ByHand, Keys) {
    let mut rng = hab_proto::opaque_ke::rand::rngs::OsRng;
    let started = ClientLogin::<Suite>::start(&mut rng, password.as_bytes()).unwrap();
    let reply: LoginStarted = rig
        .pinned()
        .post(
            "/api/v1/login/start",
            &LoginStart {
                username: "chris".into(),
                credential_request: started.message.serialize().to_vec(),
            },
        )
        .await
        .unwrap();
    let response = CredentialResponse::<Suite>::deserialize(&reply.credential_response).unwrap();
    let ksf = argon2();
    let done = started
        .state
        .finish(
            &mut rng,
            password.as_bytes(),
            response,
            ClientLoginFinishParameters::new(None, Identifiers::default(), Some(&ksf)),
        )
        .unwrap();
    let finished: LoginFinished = rig
        .pinned()
        .post(
            "/api/v1/login/finish",
            &LoginFinish {
                login_id: reply.login_id.clone(),
                credential_finalization: done.message.serialize().to_vec(),
            },
        )
        .await
        .unwrap();
    let keys = Keys::open(&finished.bundle, done.export_key.as_slice(), "chris").unwrap();
    (
        ByHand {
            login_id: reply.login_id,
            finished: Some(finished),
        },
        keys,
    )
}

fn status_of<T: std::fmt::Debug>(r: Result<T, TlsError>) -> u16 {
    match r {
        Err(TlsError::Status { status, .. }) => status,
        other => panic!("expected the server to refuse, got {other:?}"),
    }
}

#[tokio::test]
async fn the_server_adds_only_devices_the_identity_signed_after_a_proven_password() {
    let rig = rig().await;

    // No device without a proven password.
    let stranger = Keys::generate();
    let record = DeviceKeys::generate().record(&stranger.identity, true);
    let r: Result<hab_proto::wire::Joined, _> = rig
        .pinned()
        .post(
            "/api/v1/login/device",
            &LoginDevice {
                login_id: "0123456789abcdef".into(),
                device: record,
            },
        )
        .await;
    assert_eq!(status_of(r), 401);

    // A started login that isn't proven releases no bundle and adds no device.
    let mut rng = hab_proto::opaque_ke::rand::rngs::OsRng;
    let started = ClientLogin::<Suite>::start(&mut rng, b"whatever").unwrap();
    let reply: LoginStarted = rig
        .pinned()
        .post(
            "/api/v1/login/start",
            &LoginStart {
                username: "chris".into(),
                credential_request: started.message.serialize().to_vec(),
            },
        )
        .await
        .unwrap();
    let r: Result<LoginFinished, _> = rig
        .pinned()
        .post(
            "/api/v1/login/finish",
            &LoginFinish {
                login_id: reply.login_id.clone(),
                credential_finalization: vec![7; 64],
            },
        )
        .await;
    assert_eq!(status_of(r), 401);
    let record = DeviceKeys::generate().record(&stranger.identity, true);
    let r: Result<hab_proto::wire::Joined, _> = rig
        .pinned()
        .post(
            "/api/v1/login/device",
            &LoginDevice {
                login_id: reply.login_id,
                device: record,
            },
        )
        .await;
    assert_eq!(status_of(r), 401);

    // A proven password still can't add a device some other identity signed.
    let (hand, keys) = start_by_hand(&rig, &rig.password).await;
    let bad = DeviceKeys::generate().record(&stranger.identity, true);
    let r: Result<hab_proto::wire::Joined, _> = rig
        .pinned()
        .post(
            "/api/v1/login/device",
            &LoginDevice {
                login_id: hand.login_id.clone(),
                device: bad,
            },
        )
        .await;
    assert_eq!(status_of(r), 400);
    assert_eq!(rig.device_count(), 1);

    // One that its own identity signed is added, once.
    let (hand, keys2) = start_by_hand(&rig, &rig.password).await;
    assert_eq!(keys.identity_public(), keys2.identity_public());
    assert_eq!(
        hand.finished.as_ref().unwrap().identity_public,
        keys2.identity_public().to_vec()
    );
    let good = DeviceKeys::generate().record(&keys2.identity, true);
    let added: hab_proto::wire::Joined = rig
        .pinned()
        .post(
            "/api/v1/login/device",
            &LoginDevice {
                login_id: hand.login_id.clone(),
                device: good.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(added.account_id, rig.first_profile.account_id);
    assert_eq!(rig.device_count(), 2);
    let again: Result<hab_proto::wire::Joined, _> = rig
        .pinned()
        .post(
            "/api/v1/login/device",
            &LoginDevice {
                login_id: hand.login_id,
                device: good,
            },
        )
        .await;
    assert_eq!(status_of(again), 401);
    assert_eq!(rig.device_count(), 2);
    rig.server.shutdown().await;
}
