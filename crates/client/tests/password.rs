//! End to end: changing the password, and protection against password
//! guessing (backoff, the notice on the user's devices, per-address limits).

use hab_client::tls::{Pinned, TlsError};
use hab_client::{
    join, sign_in, suggest_passphrase, JoinRequest, KeyId, KeyKind, KeyStore, Profile, SignInError,
    SignInRequest, SyncError, Syncer,
};
use hab_core::Core;
use hab_proto::opaque_ke::{
    ClientLogin, ClientLoginFinishParameters, CredentialResponse, Identifiers,
};
use hab_proto::wire::{
    ApprovalCollect, ApprovalOpen, ApprovalRef, LoginFinish, LoginFinished, LoginStart,
    LoginStarted, PasswordFinish, PasswordStart,
};
use hab_proto::{argon2, Keys, Suite};
use hab_server::guard::ManualClock;
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

fn status_of<T: std::fmt::Debug>(r: Result<T, TlsError>) -> (u16, String) {
    match r {
        Err(TlsError::Status { status, message }) => (status, message),
        other => panic!("expected the server to refuse, got {other:?}"),
    }
}

/// A device that is running: its core, keys and sync loop.
struct Running {
    profile: Profile,
    store: KeyStore,
    core: Arc<Mutex<Core>>,
    syncer: Arc<Syncer>,
    wake: Arc<Notify>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Running {
    async fn start(
        profile: Profile,
        store: KeyStore,
        core: Arc<Mutex<Core>>,
        dir: tempfile::TempDir,
        signed_in_as: Option<&str>,
    ) -> Running {
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
        Running {
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

    /// Stop the loop and start it again on the same keys and core, as the
    /// app does when it is opened again.
    async fn restart(self) -> Running {
        let _ = self.stop.send(true);
        let _ = self.task.await;
        Running::start(self.profile, self.store, self.core, self._dir, None).await
    }

    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }

    fn connected(&self) -> bool {
        self.syncer.status().connected
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

    fn notices(&self) -> Vec<String> {
        self.core
            .lock()
            .unwrap()
            .snapshot()
            .security_notices
            .into_iter()
            .map(|n| n.text)
            .collect()
    }

    /// Has asked the server for notices at least once.
    fn has_baseline(&self) -> bool {
        self.core
            .lock()
            .unwrap()
            .server_notice_cursor()
            .unwrap()
            .is_some()
    }
}

struct Rig {
    server_dir: std::path::PathBuf,
    _dir: tempfile::TempDir,
    server: Server,
    clock: ManualClock,
    password: String,
    desktop: Running,
}

/// A server on a clock the test moves, with one account whose desktop has
/// joined and is syncing.
async fn rig() -> Rig {
    let server_dir = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let clock = ManualClock::new();
    let server = Server::start_with_clock(&config(server_dir.path()), clock.clock())
        .await
        .unwrap();
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
    let desktop = Running::start(profile, store, Arc::new(Mutex::new(core)), app, None).await;
    desktop.syncer.upload_standalone().await.unwrap();
    desktop.wake.notify_one();
    until("the desktop to connect", || desktop.connected()).await;
    until("the desktop to ask for notices", || desktop.has_baseline()).await;
    Rig {
        server_dir: server_dir.path().to_path_buf(),
        _dir: server_dir,
        server,
        clock,
        password,
        desktop,
    }
}

impl Rig {
    fn conn(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.server_dir.join(db::FILE_NAME)).unwrap()
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

    /// Sign in a second device and have it syncing.
    async fn laptop(&self) -> Running {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::file(dir.path());
        let signed = sign_in(self.request("Laptop", &self.password), &store)
            .await
            .unwrap();
        let core = Arc::new(Mutex::new(Core::open_in_memory().unwrap()));
        core.lock()
            .unwrap()
            .join(
                &format!("u{}", signed.profile.account_id),
                &signed.profile.device_id.to_string(),
            )
            .unwrap();
        let running = Running::start(signed.profile, store, core, dir, Some("Laptop")).await;
        until("the laptop to connect", || running.connected()).await;
        running
    }

    /// One password attempt that a guesser makes: ask the server to start a
    /// sign-in and go no further, as a client does when its own check of the
    /// password fails. Returns the server's answer.
    async fn guess(&self, username: &str) -> Result<LoginStarted, TlsError> {
        let mut rng = hab_proto::opaque_ke::rand::rngs::OsRng;
        let started = ClientLogin::<Suite>::start(&mut rng, b"a guess").unwrap();
        self.pinned()
            .post(
                "/api/v1/login/start",
                &LoginStart {
                    username: username.into(),
                    credential_request: started.message.serialize().to_vec(),
                },
            )
            .await
    }

    /// What the server keeps, apart from the password: rows that a change of
    /// password must leave alone (accounts' identity, devices, lists) and
    /// rows that devices go on adding to (sealed keys, events).
    fn everything_but_the_password(&self) -> (Vec<String>, Vec<String>) {
        let c = self.conn();
        let rows = |sqls: &[&str]| {
            let mut out = Vec::new();
            for sql in sqls {
                let mut stmt = c.prepare(sql).unwrap();
                let found = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
                out.extend(found.map(|r| r.unwrap()));
            }
            out
        };
        (
            rows(&[
                "SELECT hex(identity_public) || username || display_name || admin FROM accounts",
                "SELECT id || hex(signing_public) || hex(sealing_public) || hex(signature) FROM devices ORDER BY id",
                "SELECT id FROM lists",
            ]),
            rows(&[
                "SELECT list_id || device_id || key_version FROM list_keys",
                "SELECT list_id || seq || event_id || hex(ciphertext) FROM events",
            ]),
        )
    }

    fn password_rows(&self) -> (Vec<u8>, Vec<u8>) {
        self.conn()
            .query_row("SELECT opaque_record, bundle FROM accounts", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
    }
}

async fn account_keys(r: &Running) -> Keys {
    Keys::from_bytes(
        &r.store
            .get(&key_id(&r.profile, KeyKind::Account))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn changing_the_password_re_encrypts_the_bundle_and_nothing_else() {
    let rig = rig().await;
    let laptop = rig.laptop().await;
    let before = rig.everything_but_the_password();
    let (record_before, bundle_before) = rig.password_rows();

    // The old password is not asked for: a device that is still signed in is
    // enough, which is also what a forgotten password needs.
    let new = suggest_passphrase();
    rig.desktop.syncer.change_password(&new).await.unwrap();

    // Only the OPAQUE record and the bundle changed.
    let (record_after, bundle_after) = rig.password_rows();
    assert_ne!(record_after, record_before);
    assert_ne!(bundle_after, bundle_before);
    let after = rig.everything_but_the_password();
    assert_eq!(after.0, before.0);
    // Nothing sealed or sent before was touched; devices may have added more.
    assert!(before.1.iter().all(|row| after.1.contains(row)));

    // The old password no longer signs in; the new one does, to the same keys.
    let dir = tempfile::tempdir().unwrap();
    let store = KeyStore::file(dir.path());
    let old = sign_in(rig.request("Tablet", &rig.password), &store).await;
    assert!(matches!(old, Err(SignInError::WrongLogin)), "{old:?}");
    let tablet = sign_in(rig.request("Tablet", &new), &store).await.unwrap();
    let keys = Keys::from_bytes(
        &store
            .get(&key_id(&tablet.profile, KeyKind::Account))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let desktop_keys = account_keys(&rig.desktop).await;
    assert_eq!(keys.personal, desktop_keys.personal);
    assert_eq!(keys.identity_public(), desktop_keys.identity_public());

    // Devices that were already signed in notice nothing: they sign requests
    // with their own keys, and carry on syncing both ways.
    laptop.remind("Written on the laptop after the change");
    until("the desktop to get the laptop's reminder", || {
        rig.desktop.sees("Written on the laptop after the change")
    })
    .await;
    rig.desktop
        .remind("Written on the desktop after the change");
    until("the laptop to get the desktop's reminder", || {
        laptop.sees("Written on the desktop after the change")
    })
    .await;
    assert!(laptop.connected() && rig.desktop.connected());

    // The laptop can set another password in its turn, and then only that one works.
    let newer = suggest_passphrase();
    laptop.syncer.change_password(&newer).await.unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    let store2 = KeyStore::file(dir2.path());
    assert!(sign_in(rig.request("Phone", &new), &store2).await.is_err());
    sign_in(rig.request("Phone", &newer), &store2)
        .await
        .unwrap();

    laptop.stop().await;
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn the_new_password_needs_the_same_strength_as_joining() {
    let rig = rig().await;
    let before = (rig.password_rows(), rig.everything_but_the_password());
    for weak in ["password123", "chris", "chris-chris", "short"] {
        let r = rig.desktop.syncer.change_password(weak).await;
        assert!(matches!(r, Err(SyncError::WeakPassword)), "{weak}: {r:?}");
    }
    // The server stored nothing, and the old password still works.
    assert_eq!(
        (rig.password_rows(), rig.everything_but_the_password()),
        before
    );
    let dir = tempfile::tempdir().unwrap();
    sign_in(
        rig.request("Laptop", &rig.password),
        &KeyStore::file(dir.path()),
    )
    .await
    .unwrap();
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn a_password_can_only_be_changed_by_a_signed_in_device() {
    let rig = rig().await;
    let (record, bundle) = rig.password_rows();
    let mut rng = hab_proto::opaque_ke::rand::rngs::OsRng;
    let started =
        hab_proto::opaque_ke::ClientRegistration::<Suite>::start(&mut rng, b"whatever").unwrap();
    let r: Result<serde_json::Value, _> = rig
        .pinned()
        .post(
            "/api/v1/password/start",
            &PasswordStart {
                registration_request: started.message.serialize().to_vec(),
            },
        )
        .await;
    assert_eq!(status_of(r).0, 401);
    let r: Result<serde_json::Value, _> = rig
        .pinned()
        .post(
            "/api/v1/password/finish",
            &PasswordFinish {
                registration_upload: vec![1; 32],
                kdf: hab_proto::wire::Kdf {
                    alg: "argon2id".into(),
                    memory_kib: hab_proto::ARGON_MEMORY_KIB,
                    passes: hab_proto::ARGON_PASSES,
                    lanes: hab_proto::ARGON_LANES,
                },
                bundle: Keys::generate().seal(b"x", "chris"),
            },
        )
        .await;
    assert_eq!(status_of(r).0, 401);
    assert_eq!(rig.password_rows(), (record, bundle));
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn a_sign_in_under_way_cannot_finish_with_the_old_password_after_a_change() {
    let rig = rig().await;
    // Start and prove a sign-in with the old password, but hold back the
    // last step.
    let mut rng = hab_proto::opaque_ke::rand::rngs::OsRng;
    let started = ClientLogin::<Suite>::start(&mut rng, rig.password.as_bytes()).unwrap();
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
            rig.password.as_bytes(),
            response,
            ClientLoginFinishParameters::new(None, Identifiers::default(), Some(&ksf)),
        )
        .unwrap();

    rig.desktop
        .syncer
        .change_password(&suggest_passphrase())
        .await
        .unwrap();

    let r: Result<LoginFinished, _> = rig
        .pinned()
        .post(
            "/api/v1/login/finish",
            &LoginFinish {
                login_id: reply.login_id,
                credential_finalization: done.message.serialize().to_vec(),
            },
        )
        .await;
    assert_eq!(status_of(r).0, 401);
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[tokio::test]
async fn after_five_failures_each_attempt_waits_twice_as_long_and_the_devices_are_told() {
    let rig = rig().await;
    let laptop = rig.laptop().await;
    until("the laptop to ask for notices", || laptop.has_baseline()).await;
    // A device that is closed while it happens, and hears of it when it opens.
    let sleeper = rig.laptop().await;
    until("the second laptop to ask for notices", || {
        sleeper.has_baseline()
    })
    .await;
    let _ = sleeper.stop.send(true);

    // Failures at the approvals' unauthenticated routes are not failed
    // sign-ins, whoever makes them.
    let pinned = rig.pinned();
    for i in 0..30 {
        let r: Result<serde_json::Value, _> = pinned
            .post(
                "/api/v1/approvals/collect",
                &ApprovalCollect {
                    approval_id: format!("{i:032x}"),
                    collect_token: "t".repeat(32),
                },
            )
            .await;
        assert_eq!(status_of(r).0, 404);
    }

    // Five attempts that go no further than the server's first answer: the
    // password was checked by the guesser, who saw it fail.
    for _ in 0..5 {
        rig.guess("chris").await.unwrap();
    }
    // Now the account waits. Even the right password is refused, in the same
    // way as any other failure, and the attempt is not counted.
    let dir = tempfile::tempdir().unwrap();
    let store = KeyStore::file(dir.path());
    let refused = sign_in(rig.request("Tablet", &rig.password), &store).await;
    assert!(
        matches!(refused, Err(SignInError::WrongLogin)),
        "{refused:?}"
    );
    rig.clock.advance(secs(58));
    let refused = sign_in(rig.request("Tablet", &rig.password), &store).await;
    assert!(
        matches!(refused, Err(SignInError::WrongLogin)),
        "{refused:?}"
    );

    // Nothing is said yet: the fifth attempt might still have succeeded.
    assert!(laptop.notices().is_empty());

    // A minute after the fifth, the sixth is allowed to count. It fails too.
    rig.clock.advance(secs(2));
    rig.guess("chris").await.unwrap();
    // The seventh waits two minutes from the sixth.
    rig.clock.advance(secs(119));
    let refused = sign_in(rig.request("Tablet", &rig.password), &store).await;
    assert!(
        matches!(refused, Err(SignInError::WrongLogin)),
        "{refused:?}"
    );

    // The user's devices show it, connected or not.
    until("the laptop to show the notice", || {
        laptop.notices() == ["5 failed sign-ins to your account"]
    })
    .await;
    until("the desktop to show the notice", || {
        rig.desktop.notices() == ["5 failed sign-ins to your account"]
    })
    .await;
    let sleeper = sleeper.restart().await;
    until("the closed device to show the notice", || {
        sleeper.notices() == ["5 failed sign-ins to your account"]
    })
    .await;
    // One notice, however many more attempts.
    assert_eq!(
        rig.conn()
            .query_row("SELECT COUNT(*) FROM notices", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );

    // The wait has passed: the right password signs in, and wipes the count.
    rig.clock.advance(secs(1));
    let tablet = sign_in(rig.request("Tablet", &rig.password), &store)
        .await
        .unwrap();
    assert_eq!(tablet.profile.username, "chris");
    for _ in 0..4 {
        rig.guess("chris").await.unwrap();
    }
    rig.clock.advance(secs(1));
    sign_in(
        rig.request("Phone", &rig.password),
        &KeyStore::file(tempfile::tempdir().unwrap().path()),
    )
    .await
    .unwrap();

    sleeper.stop().await;
    laptop.stop().await;
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn the_wait_doubles_to_an_hour_and_stops_there() {
    let rig = rig().await;
    for _ in 0..5 {
        rig.guess("chris").await.unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let store = KeyStore::file(dir.path());
    let mut wait = 60;
    let mut waits = Vec::new();
    // Each attempt is made at the first moment it is allowed; one second
    // earlier is refused, and the next wait is twice as long.
    for _ in 0..9 {
        waits.push(wait);
        rig.clock.advance(secs(wait));
        rig.guess("chris").await.unwrap();
        wait = (wait * 2).min(3600);
    }
    assert_eq!(waits, [60, 120, 240, 480, 960, 1920, 3600, 3600, 3600]);

    // The ninth wait was an hour; one second short of the next is refused,
    // then it works and the right password signs in.
    rig.clock.advance(secs(3599));
    let refused = sign_in(rig.request("Tablet", &rig.password), &store).await;
    assert!(
        matches!(refused, Err(SignInError::WrongLogin)),
        "{refused:?}"
    );
    rig.clock.advance(secs(1));
    sign_in(rig.request("Tablet", &rig.password), &store)
        .await
        .unwrap();
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn a_waiting_account_looks_like_an_unknown_username() {
    let rig = rig().await;
    for _ in 0..5 {
        rig.guess("chris").await.unwrap();
    }
    let waiting = rig.guess("chris").await.unwrap();
    let stranger = rig.guess("nobody").await.unwrap();
    // The same shape: the same sizes, hardening and kind of id.
    assert_eq!(
        waiting.credential_response.len(),
        stranger.credential_response.len()
    );
    assert_eq!(waiting.kdf, stranger.kdf);
    assert_eq!(waiting.login_id.len(), stranger.login_id.len());

    // Finishing either is the same refusal.
    let finish = |id: String| LoginFinish {
        login_id: id,
        credential_finalization: vec![7; 64],
    };
    let a: Result<LoginFinished, _> = rig
        .pinned()
        .post("/api/v1/login/finish", &finish(waiting.login_id))
        .await;
    let b: Result<LoginFinished, _> = rig
        .pinned()
        .post("/api/v1/login/finish", &finish(stranger.login_id))
        .await;
    assert_eq!(status_of(a), status_of(b));
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn an_address_gets_twenty_password_attempts_a_minute_counted_in_memory() {
    let rig = rig().await;
    // Twenty, for an account that exists and for one that doesn't.
    for i in 0..20 {
        let name = if i % 2 == 0 { "chris" } else { "nobody" };
        rig.guess(name).await.unwrap();
    }
    let (status, message) = status_of(rig.guess("nobody").await);
    assert_eq!(status, 429);
    assert!(message.contains("too many"), "{message}");
    // The right password from this address is refused too, with a message
    // that says so.
    let dir = tempfile::tempdir().unwrap();
    let store = KeyStore::file(dir.path());
    let refused = sign_in(rig.request("Tablet", &rig.password), &store).await;
    assert!(
        matches!(refused, Err(SignInError::TooManyAttempts)),
        "{refused:?}"
    );

    // Not the approvals' calls: they count apart.
    let r: Result<serde_json::Value, _> = rig
        .pinned()
        .post(
            "/api/v1/approvals/collect",
            &ApprovalCollect {
                approval_id: "0".repeat(32),
                collect_token: "t".repeat(32),
            },
        )
        .await;
    assert_eq!(status_of(r).0, 404);

    // A minute on, the address is let in again.
    rig.clock.advance(secs(60));
    sign_in(rig.request("Tablet", &rig.password), &store)
        .await
        .unwrap();

    // Nothing about addresses is kept: not in the database file, and no
    // table or column made for one.
    let c = rig.conn();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(!tables
        .iter()
        .any(|t| t.contains("ip") || t.contains("addr")));
    let columns: Vec<String> = c
        .prepare("SELECT m.name || '.' || p.name FROM sqlite_master m, pragma_table_info(m.name) p WHERE m.type = 'table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(
        !columns.iter().any(|c| {
            let c = c.to_lowercase();
            c.contains("addr") || c.ends_with(".ip") || c.contains("_ip")
        }),
        "{columns:?}"
    );
    drop(c);
    let file = std::fs::read(rig.server_dir.join(db::FILE_NAME)).unwrap();
    assert!(!file.windows(9).any(|w| w == b"127.0.0.1"));

    rig.desktop.stop().await;
    rig.server.shutdown().await;
}

#[tokio::test]
async fn the_approvals_unauthenticated_routes_have_an_address_limit_too() {
    let rig = rig().await;
    let pinned = rig.pinned();
    let call = |i: usize| {
        let pinned = pinned.clone();
        async move {
            let r: Result<serde_json::Value, _> = pinned
                .post(
                    "/api/v1/approvals/collect",
                    &ApprovalCollect {
                        approval_id: format!("{i:032x}"),
                        collect_token: "t".repeat(32),
                    },
                )
                .await;
            r
        }
    };
    for i in 0..hab_server::guard::IP_APPROVAL_CALLS_PER_MINUTE {
        let r = call(i).await;
        assert_eq!(status_of(r).0, 404, "call {i}");
    }
    assert_eq!(status_of(call(9999).await).0, 429);
    // The other unauthenticated routes share the bucket.
    let open: Result<ApprovalRef, _> = pinned
        .post(
            "/api/v1/approvals/open",
            &ApprovalOpen {
                approval_id: "0".repeat(32),
                request: hab_proto::wire::ApprovalBlob {
                    alg: hab_proto::wire::Algs::APPROVAL.into(),
                    nonce: vec![0; 24],
                    ciphertext: vec![],
                },
                collect_token: "t".repeat(32),
            },
        )
        .await;
    assert_eq!(status_of(open).0, 429);
    // Password attempts are counted apart and still go through.
    rig.guess("chris").await.unwrap();
    // Anything refused here was no failed sign-in: five more fresh attempts
    // are all free.
    rig.clock.advance(secs(61));
    assert_eq!(status_of(call(1).await).0, 404);
    rig.desktop.stop().await;
    rig.server.shutdown().await;
}
