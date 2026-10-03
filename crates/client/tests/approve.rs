//! End to end: approving a new device from an existing one by QR code or link.

use hab_client::tls::{Pinned, TlsError};
use hab_client::{
    join, parse_target, sign_in, suggest_passphrase, ApprovalLink, JoinRequest, KeyId, KeyKind,
    KeyStore, NewDevice, Profile, SignInRequest, SignInTarget, SyncError, Syncer,
};
use hab_core::Core;
use hab_proto::wire::{
    ApprovalBlob, ApprovalCollect, ApprovalCollected, ApprovalFetched, ApprovalGrant, ApprovalOpen,
    ApprovalRef, ApprovalRequest, Joined,
};
use hab_proto::{sign_device, ApprovalKey, DeviceKeys, Keys};
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

const WAIT: Duration = Duration::from_secs(20);

impl Rig {
    /// Start a device that was approved, as the app does: it takes the
    /// account's list and announces itself.
    async fn start_approved(
        &self,
        profile: Profile,
        store: KeyStore,
        dir: tempfile::TempDir,
    ) -> Device {
        let mut core = Core::open_in_memory().unwrap();
        core.join(
            &format!("u{}", profile.account_id),
            &profile.device_id.to_string(),
        )
        .unwrap();
        let name = profile.device_name.clone();
        let device =
            Device::start(profile, store, Arc::new(Mutex::new(core)), dir, Some(&name)).await;
        until(&format!("{name} to connect"), || {
            device.syncer.status().connected
        })
        .await;
        device
    }

    fn address(&self) -> String {
        self.server.local_addr().to_string()
    }

    fn file_holds(&self, needle: &str) -> bool {
        let file = std::fs::read(self.server_dir.path().join(db::FILE_NAME)).unwrap();
        file.windows(needle.len()).any(|w| w == needle.as_bytes())
    }
}

fn approval_link(link: &ApprovalLink) -> ApprovalLink {
    // What the other device gets: the text of the link, as scanned or pasted.
    match parse_target(&link.link()).unwrap() {
        SignInTarget::Approval(l) => l,
        other => panic!("expected an approval link, got {other:?}"),
    }
}

/// Rotate the personal list's key, so that approval has more than the first
/// version to seal, and make a reminder with the new key.
async fn rotate_and_remind(rig: &Rig, title: &str) {
    let phone = rig.sign_in("Phone").await;
    let phone_id = phone.id();
    phone.stop().await;
    rig.desktop.syncer.remove_device(phone_id).await.unwrap();
    rig.desktop.remind(title);
    until("the reminder to be sent", || {
        !rig.desktop
            .core
            .lock()
            .unwrap()
            .snapshot()
            .upcoming
            .iter()
            .any(|u| u.title == title && u.not_sent)
    })
    .await;
}

#[tokio::test]
async fn an_existing_device_shows_a_code_and_the_new_device_scans_it() {
    let rig = rig().await;
    let desktop = &rig.desktop;
    rotate_and_remind(&rig, "After the rotation").await;

    let link = desktop.syncer.offer_approval().await.unwrap();
    // The new device only has the text of the link, from a camera or a paste.
    assert!(link.qr_svg().contains("<svg"));
    let scanned = approval_link(&link);
    assert_eq!(scanned, link);
    assert!(desktop
        .syncer
        .pending_device(&link)
        .await
        .unwrap()
        .is_none());

    let app = tempfile::tempdir().unwrap();
    let store = KeyStore::file(app.path());
    let new = NewDevice::scan(&scanned, "Tablet", true).await.unwrap();
    // Nothing is approved, so there is nothing to collect yet.
    assert!(new.collect(&store).await.unwrap().is_none());
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM devices WHERE account_id = ?1", 1),
        1
    );

    // The existing device shows the new device's name and waits for a yes.
    let pending = desktop
        .syncer
        .pending_device(&link)
        .await
        .unwrap()
        .expect("the request has arrived");
    assert_eq!(pending.name, "Tablet");
    assert!(pending.portable);
    let new_id = desktop.syncer.approve_device(&pending).await.unwrap();

    let signed = new.wait(&store, WAIT).await.unwrap();
    let profile = signed.profile;
    assert_eq!(profile.device_id, new_id);
    assert_eq!(profile.username, "chris");
    assert_eq!(profile.display_name, "Chris");
    assert_eq!(profile.device_name, "Tablet");
    assert_eq!(profile.server_fingerprint, rig.server.fingerprint());

    // The identity key vouches for it, and the account's keys came across.
    let account_keys = desktop.account_keys().await;
    let stored = Keys::from_bytes(
        &store
            .get(&key_id(&profile, KeyKind::Account))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(stored.identity_public(), account_keys.identity_public());
    assert_eq!(stored.personal, account_keys.personal);
    let listed = desktop.syncer.devices().await.unwrap();
    assert!(listed.iter().any(|d| d.id == new_id && !d.this_device));

    // Every version of the list key is already sealed to it, by the approver.
    assert_eq!(
        rig.sealed_to(new_id),
        vec![(1, desktop.id()), (2, desktop.id())]
    );

    // It syncs as after a password sign-in: it reads what was made after the
    // rotation straight away, and the other devices are told.
    let tablet = rig.start_approved(profile, store, app).await;
    until("the tablet to get the desktop's reminders", || {
        tablet.sees("After the rotation") && tablet.sees("Water the plants")
    })
    .await;
    until("the desktop to hear of the tablet", || {
        desktop
            .core
            .lock()
            .unwrap()
            .snapshot()
            .sign_in_notices
            .iter()
            .any(|n| n.device_name == "Tablet" && n.device_id == new_id.to_string())
    })
    .await;
    tablet.remind("From the tablet");
    until("the desktop to get the tablet's reminder", || {
        desktop.sees("From the tablet")
    })
    .await;
    assert!(tablet
        .core
        .lock()
        .unwrap()
        .snapshot()
        .sign_in_notices
        .is_empty());

    // The server saw neither the name, nor the approval key, nor the keys.
    for needle in [
        "Tablet".to_string(),
        link.key.to_text(),
        rig.password.clone(),
    ] {
        assert!(
            !rig.file_holds(&needle),
            "the server's file holds {needle:?}"
        );
    }

    // The approval is used up.
    assert!(matches!(
        new.collect(&KeyStore::file(tempfile::tempdir().unwrap().path()))
            .await,
        Err(hab_client::ApproveError::Expired)
    ));
    assert!(matches!(
        desktop.syncer.pending_device(&link).await,
        Err(SyncError::ApprovalExpired)
    ));

    tablet.stop().await;
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn the_new_device_shows_a_code_and_the_existing_device_scans_it() {
    let rig = rig().await;
    let desktop = &rig.desktop;
    rotate_and_remind(&rig, "After the rotation").await;

    let app = tempfile::tempdir().unwrap();
    let store = KeyStore::file(app.path());
    let new = NewDevice::show(
        &rig.address(),
        rig.server.fingerprint(),
        "Home server",
        "Phone 2",
        true,
    )
    .await
    .unwrap();
    let scanned = approval_link(new.link());

    let pending = desktop
        .syncer
        .pending_device(&scanned)
        .await
        .unwrap()
        .expect("the new device sent its request when it made the code");
    assert_eq!(pending.name, "Phone 2");
    assert!(new.collect(&store).await.unwrap().is_none());
    let new_id = desktop.syncer.approve_device(&pending).await.unwrap();

    let profile = new.wait(&store, WAIT).await.unwrap().profile;
    assert_eq!(profile.device_id, new_id);
    assert_eq!(profile.device_name, "Phone 2");
    assert_eq!(
        rig.sealed_to(new_id),
        vec![(1, desktop.id()), (2, desktop.id())]
    );

    let phone = rig.start_approved(profile, store, app).await;
    until("the new phone to get the desktop's reminders", || {
        phone.sees("After the rotation")
    })
    .await;

    // A device approved this way can approve the next one itself.
    let link = phone.syncer.offer_approval().await.unwrap();
    let app3 = tempfile::tempdir().unwrap();
    let store3 = KeyStore::file(app3.path());
    let third = NewDevice::scan(&approval_link(&link), "Laptop", false)
        .await
        .unwrap();
    let pending = phone.syncer.pending_device(&link).await.unwrap().unwrap();
    phone.syncer.approve_device(&pending).await.unwrap();
    let profile3 = third.wait(&store3, WAIT).await.unwrap().profile;
    let laptop = rig.start_approved(profile3, store3, app3).await;
    until("the laptop to get the reminders", || {
        laptop.sees("After the rotation")
    })
    .await;

    laptop.stop().await;
    phone.stop().await;
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn a_code_works_once_and_only_for_those_who_hold_all_of_it() {
    let rig = rig().await;
    let desktop = &rig.desktop;
    let link = desktop.syncer.offer_approval().await.unwrap();

    // The key is in the code only: without it a request can't be read, and
    // the existing device refuses it.
    let impostor = ApprovalLink {
        key: ApprovalKey::generate(),
        ..link.clone()
    };
    let _first = NewDevice::scan(&impostor, "Impostor", true).await.unwrap();
    assert!(matches!(
        desktop.syncer.pending_device(&link).await,
        Err(SyncError::BadApproval(_))
    ));
    // And its request holds the place: a second device can't take it over.
    assert!(matches!(
        NewDevice::scan(&link, "Tablet", true).await,
        Err(hab_client::ApproveError::Expired)
    ));
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM devices WHERE account_id = ?1", 1),
        1
    );

    // By hand, against the server.
    let pinned = rig.pinned();
    let blob = |n: u8| ApprovalBlob {
        alg: hab_proto::wire::Algs::APPROVAL.into(),
        nonce: vec![n; 24],
        ciphertext: vec![n; 40],
    };
    let token = "t".repeat(32);

    // An id nobody made, and a token that is not the one sent: the same no.
    let nothing: Result<ApprovalCollected, _> = pinned
        .post(
            "/api/v1/approvals/collect",
            &ApprovalCollect {
                approval_id: "0".repeat(32),
                collect_token: token.clone(),
            },
        )
        .await;
    assert_eq!(status_of(nothing), 404);
    let opened: ApprovalRef = pinned
        .post(
            "/api/v1/approvals/open",
            &ApprovalOpen {
                approval_id: "ab".repeat(16),
                request: blob(1),
                collect_token: token.clone(),
            },
        )
        .await
        .unwrap();
    let wrong: Result<ApprovalCollected, _> = pinned
        .post(
            "/api/v1/approvals/collect",
            &ApprovalCollect {
                approval_id: opened.approval_id.clone(),
                collect_token: "u".repeat(32),
            },
        )
        .await;
    assert_eq!(status_of(wrong), 404);
    // An id can't be taken twice, nor a request added to one that has it,
    // and badly formed ones are refused.
    for (id, token) in [
        ("ab".repeat(16), token.as_str()),
        ("short".into(), token.as_str()),
        ("cd".repeat(16), "x"),
    ] {
        let again: Result<ApprovalRef, _> = pinned
            .post(
                "/api/v1/approvals/open",
                &ApprovalOpen {
                    approval_id: id,
                    request: blob(2),
                    collect_token: token.into(),
                },
            )
            .await;
        assert!(matches!(status_of(again), 400 | 409));
    }
    let added: Result<serde_json::Value, _> = pinned
        .post(
            "/api/v1/approvals/request",
            &ApprovalRequest {
                approval_id: opened.approval_id.clone(),
                request: blob(3),
                collect_token: token.clone(),
            },
        )
        .await;
    assert_eq!(status_of(added), 404);

    // Approving needs a signed-in device, and the identity key's signature on
    // the device: one signed by any other key is refused, and nothing is added.
    let desktop_keys = desktop.device_keys().await;
    let fetched: ApprovalFetched = pinned
        .signed_post(
            "/api/v1/approvals/fetch",
            &ApprovalRef {
                approval_id: opened.approval_id.clone(),
            },
            desktop.id(),
            &desktop_keys,
        )
        .await
        .unwrap();
    assert_eq!(fetched.request, Some(blob(1)));
    let own = DeviceKeys::generate();
    let forged = sign_device(
        &Keys::generate().identity,
        &own.signing.verifying_key().to_bytes(),
        &own.sealing_public(),
        true,
    );
    let grant = |device| ApprovalGrant {
        approval_id: opened.approval_id.clone(),
        device,
        grant: blob(4),
    };
    let refused: Result<Joined, _> = pinned
        .signed_post(
            "/api/v1/approvals/grant",
            &grant(forged),
            desktop.id(),
            &desktop_keys,
        )
        .await;
    assert_eq!(status_of(refused), 400);
    let unsigned: Result<Joined, _> = pinned
        .post(
            "/api/v1/approvals/grant",
            &grant(own.record(&Keys::generate().identity, true)),
        )
        .await;
    assert_eq!(status_of(unsigned), 401);
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM devices WHERE account_id = ?1", 1),
        1
    );

    // A good grant is used once; the answer is collected once.
    let identity = desktop.account_keys().await.identity.clone();
    let signed = sign_device(
        &identity,
        &own.signing.verifying_key().to_bytes(),
        &own.sealing_public(),
        true,
    );
    let joined: Joined = pinned
        .signed_post(
            "/api/v1/approvals/grant",
            &grant(signed.clone()),
            desktop.id(),
            &desktop_keys,
        )
        .await
        .unwrap();
    let twice: Result<Joined, _> = pinned
        .signed_post(
            "/api/v1/approvals/grant",
            &grant(signed),
            desktop.id(),
            &desktop_keys,
        )
        .await;
    assert_eq!(status_of(twice), 404);
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM devices WHERE account_id = ?1", 1),
        2
    );
    let collect = ApprovalCollect {
        approval_id: opened.approval_id.clone(),
        collect_token: token,
    };
    let got: ApprovalCollected = pinned
        .post("/api/v1/approvals/collect", &collect)
        .await
        .unwrap();
    let got = got.granted.expect("the answer is there");
    assert_eq!(got.joined, joined);
    assert_eq!(got.grant, blob(4));
    let again: Result<ApprovalCollected, _> =
        pinned.post("/api/v1/approvals/collect", &collect).await;
    assert_eq!(status_of(again), 404);

    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn an_answer_not_sealed_under_the_codes_key_is_not_taken() {
    // A dishonest server can relay anything, but it does not have the key
    // that is in the code, so it can't make the new device take an account
    // of its own, nor the account's keys.
    let rig = rig().await;
    let desktop = &rig.desktop;
    let app = tempfile::tempdir().unwrap();
    let store = KeyStore::file(app.path());
    let new = NewDevice::show(
        &rig.address(),
        rig.server.fingerprint(),
        "Home server",
        "Phone 2",
        true,
    )
    .await
    .unwrap();
    let link = approval_link(new.link());
    // What the new device asked for, read with the key from the code.
    let fetched: ApprovalFetched = rig
        .pinned()
        .signed_post(
            "/api/v1/approvals/fetch",
            &ApprovalRef {
                approval_id: link.id.clone(),
            },
            desktop.id(),
            &desktop.device_keys().await,
        )
        .await
        .unwrap();
    let asked: hab_proto::wire::ApprovalRequestPlain = serde_json::from_slice(
        &link
            .key
            .open(&link.id, "request", &fetched.request.unwrap())
            .unwrap(),
    )
    .unwrap();

    // The server's own account, whose identity signs the new device's keys,
    // sealed under a key the server made up.
    let evil = Keys::generate();
    let plain = hab_proto::wire::ApprovalGrantPlain {
        username: "chris".into(),
        display_name: "Chris".into(),
        keys: evil.to_bytes(),
    };
    let other = ApprovalKey::generate();
    let grant = other.seal(&link.id, "grant", &serde_json::to_vec(&plain).unwrap());
    let desktop_keys = desktop.device_keys().await;
    let identity = desktop.account_keys().await.identity.clone();
    // The account's identity signs whatever keys it is shown, so the server
    // can't be the one to make the signature; the desktop's is used here to
    // reach the point where only the sealing matters.
    let _: Joined = rig
        .pinned()
        .signed_post(
            "/api/v1/approvals/grant",
            &ApprovalGrant {
                approval_id: link.id.clone(),
                device: sign_device(
                    &identity,
                    &asked.signing_public.clone().try_into().unwrap(),
                    &asked.sealing_public.clone().try_into().unwrap(),
                    true,
                ),
                grant,
            },
            desktop.id(),
            &desktop_keys,
        )
        .await
        .unwrap();
    let result = new.collect(&store).await;
    assert!(
        matches!(result, Err(hab_client::ApproveError::Invalid(_))),
        "{:?}",
        result.err()
    );
    // Nothing was kept on the device.
    for kind in [KeyKind::Account, KeyKind::Device] {
        let id = KeyId {
            server_fingerprint: rig.server.fingerprint().to_string(),
            username: "chris".into(),
            kind,
        };
        assert!(store.get(&id).await.unwrap().is_none());
    }

    rig.desktop.stop().await;
    rig.server.shutdown().await;
}
