//! End to end: the app side joining a real server over pinned TLS.

use hab_client::tls::{probe, Pinned, TlsError};
use hab_client::{
    join, suggest_passphrase, JoinError, JoinRequest, KeyId, KeyKind, KeyStore, Profile,
};
use hab_proto::{verify_device, DeviceKeys, Keys};
use hab_server::{db, Config, Server};
use std::path::Path;

fn config(dir: &Path) -> Config {
    Config {
        data_dir: dir.to_path_buf(),
        listen: "127.0.0.1:0".parse().unwrap(),
        name: Some("Home server".into()),
        ..Config::default()
    }
}

fn request(server: &Server, code: &str, username: &str, password: &str) -> JoinRequest {
    JoinRequest {
        address: server.local_addr().to_string(),
        fingerprint: server.fingerprint().to_string(),
        server_name: server.name().to_string(),
        setup_code: code.into(),
        username: username.into(),
        display_name: "Chris".into(),
        password: password.into(),
        device_name: "Desktop".into(),
        portable: false,
    }
}

fn account_row(dir: &Path) -> (String, bool, Vec<u8>, Vec<u8>, Vec<u8>, i64) {
    let conn = rusqlite::Connection::open(dir.join(db::FILE_NAME)).unwrap();
    conn.query_row(
        "SELECT display_name, admin, identity_public, bundle, opaque_record,
                (SELECT COUNT(*) FROM devices) FROM accounts WHERE username = 'chris'",
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
    .unwrap()
}

#[tokio::test]
async fn the_first_start_pins_the_fingerprint_then_joins_with_the_setup_code() {
    let server_dir = tempfile::tempdir().unwrap();
    let app_dir = tempfile::tempdir().unwrap();
    let server = Server::start(&config(server_dir.path())).await.unwrap();
    let addr = server.local_addr().to_string();

    // The fingerprint shown to the user is the one the server printed, and
    // the info is read over a connection pinned to it.
    let fp = probe(&addr).await.unwrap();
    assert_eq!(fp, server.fingerprint());
    let info = Pinned::new(&addr, &fp).info().await.unwrap();
    assert_eq!(info.name, "Home server");

    let password = suggest_passphrase();
    let store = KeyStore::file(app_dir.path());
    let joined = join(
        request(&server, &server.setup_code(), "chris", &password),
        &store,
    )
    .await
    .unwrap();
    let Profile {
        username,
        admin,
        server_fingerprint,
        ..
    } = &joined.profile;
    assert_eq!(username, "chris");
    assert!(*admin, "the first account is the server admin");
    assert_eq!(server_fingerprint, &fp);

    // The server holds the account, its one device, and a bundle it can't read.
    let (display, admin, identity_public, bundle, record, devices) = account_row(server_dir.path());
    assert_eq!((display.as_str(), admin, devices), ("Chris", true, 1));

    // The device kept its keys, and the identity key it kept is the account's.
    let id = |kind| KeyId {
        server_fingerprint: fp.clone(),
        username: "chris".into(),
        kind,
    };
    let keys = Keys::from_bytes(&store.get(&id(KeyKind::Account)).await.unwrap().unwrap()).unwrap();
    assert_eq!(keys.identity_public().to_vec(), identity_public);
    DeviceKeys::from_bytes(&store.get(&id(KeyKind::Device)).await.unwrap().unwrap()).unwrap();
    assert!(!bundle.windows(32).any(|w| w == keys.personal.as_slice()));

    // The server never saw the password: not in the file, nor in the record.
    let file = std::fs::read(server_dir.path().join(db::FILE_NAME)).unwrap();
    for needle in [password.as_bytes(), password.replace(' ', "").as_bytes()] {
        assert!(!file.windows(needle.len()).any(|w| w == needle));
        assert!(!record.windows(needle.len()).any(|w| w == needle));
    }

    // The code stops working once the account exists.
    assert!(!server.check_setup_code(&server.setup_code()).unwrap());
    let again = join(
        request(
            &server,
            &server.setup_code(),
            "other",
            &suggest_passphrase(),
        ),
        &store,
    )
    .await;
    assert!(matches!(
        again,
        Err(JoinError::Server(TlsError::Status { status: 403, .. }))
    ));
    server.shutdown().await;
}

#[tokio::test]
async fn a_wrong_setup_code_creates_nothing_and_leaves_no_keys() {
    let server_dir = tempfile::tempdir().unwrap();
    let app_dir = tempfile::tempdir().unwrap();
    let server = Server::start(&config(server_dir.path())).await.unwrap();
    let store = KeyStore::file(app_dir.path());

    let err = join(
        request(&server, "0000-0000", "chris", &suggest_passphrase()),
        &store,
    )
    .await;
    assert!(matches!(
        err,
        Err(JoinError::Server(TlsError::Status { status: 403, .. }))
    ));
    assert!(!server.check_setup_code("0000").unwrap());
    assert!(
        !app_dir.path().join("keys").exists()
            || std::fs::read_dir(app_dir.path().join("keys"))
                .unwrap()
                .count()
                == 0
    );

    // The real code still works afterwards.
    join(
        request(
            &server,
            &server.setup_code(),
            "chris",
            &suggest_passphrase(),
        ),
        &store,
    )
    .await
    .unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn a_weak_password_never_reaches_the_server() {
    let server_dir = tempfile::tempdir().unwrap();
    let app_dir = tempfile::tempdir().unwrap();
    let server = Server::start(&config(server_dir.path())).await.unwrap();
    let store = KeyStore::file(app_dir.path());
    let err = join(
        request(&server, &server.setup_code(), "chris", "password123"),
        &store,
    )
    .await;
    assert!(matches!(err, Err(JoinError::Input(_))));
    server.shutdown().await;
}

#[tokio::test]
async fn a_server_with_a_different_certificate_is_refused() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let a = Server::start(&config(one.path())).await.unwrap();
    let b = Server::start(&config(two.path())).await.unwrap();
    // Pinned to a's fingerprint but talking to b.
    let err = Pinned::new(&b.local_addr().to_string(), a.fingerprint())
        .info()
        .await;
    assert!(matches!(err, Err(TlsError::WrongCertificate)), "{err:?}");
    assert!(Pinned::new(&a.local_addr().to_string(), a.fingerprint())
        .info()
        .await
        .is_ok());
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test]
async fn two_joins_at_once_make_one_account() {
    let server_dir = tempfile::tempdir().unwrap();
    let app_dir = tempfile::tempdir().unwrap();
    let server = Server::start(&config(server_dir.path())).await.unwrap();
    let store = KeyStore::file(app_dir.path());
    let code = server.setup_code();
    let (a, b) = tokio::join!(
        join(
            request(&server, &code, "chris", &suggest_passphrase()),
            &store
        ),
        join(
            request(&server, &code, "dana", &suggest_passphrase()),
            &store
        ),
    );
    assert_eq!(a.is_ok() as u8 + b.is_ok() as u8, 1);
    let conn = rusqlite::Connection::open(server_dir.path().join(db::FILE_NAME)).unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    server.shutdown().await;
}

#[test]
fn our_fingerprint_matches_the_servers() {
    let der = b"some certificate bytes";
    assert_eq!(
        hab_client::tls::fingerprint(der),
        hab_server::cert::fingerprint(der)
    );
}

#[test]
fn a_device_record_from_join_verifies_against_the_identity() {
    let keys = Keys::generate();
    let record = DeviceKeys::generate().record(&keys.identity, true);
    verify_device(&keys.identity_public(), &record).unwrap();
}
