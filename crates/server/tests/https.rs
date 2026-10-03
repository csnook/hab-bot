//! End to end: a real TLS client against a running server.

use hab_server::{cert, db, Config, Server};
use rustls::pki_types::{CertificateDer, ServerName};
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsConnector;

fn config(dir: &Path, debug: bool) -> Config {
    Config {
        data_dir: dir.to_path_buf(),
        listen: "127.0.0.1:0".parse().unwrap(),
        debug,
        ..Config::default()
    }
}

/// A client that trusts exactly one certificate, as an app does after pinning
/// the fingerprint from an invite.
fn connector(trusted: &CertificateDer<'static>) -> TlsConnector {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(trusted.clone()).unwrap();
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    TlsConnector::from(Arc::new(cfg))
}

/// Returns the fingerprint of the certificate presented and the response.
async fn get(
    addr: SocketAddr,
    trusted: &CertificateDer<'static>,
    path: &str,
) -> io::Result<(String, String)> {
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let mut tls = connector(trusted)
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await?;
    let presented = tls.get_ref().1.peer_certificates().unwrap()[0].clone();
    tls.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await?;
    let mut body = String::new();
    let _ = tls.read_to_string(&mut body).await; // close_notify may be missing
    Ok((cert::fingerprint(&presented), body))
}

/// POST a JSON body and return the status line and the response body.
async fn post(
    addr: SocketAddr,
    trusted: &CertificateDer<'static>,
    path: &str,
    json: &str,
) -> io::Result<(String, String)> {
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let mut tls = connector(trusted)
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await?;
    tls.write_all(
        format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{json}",
            json.len()
        )
        .as_bytes(),
    )
    .await?;
    let mut response = String::new();
    let _ = tls.read_to_string(&mut response).await;
    let status = response.lines().next().unwrap_or_default().to_string();
    let body = response
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or_default()
        .to_string();
    Ok((status, body))
}

fn stored_cert(dir: &Path) -> CertificateDer<'static> {
    let db = db::Db::open(dir).unwrap();
    cert::load_or_create(&db, &[]).unwrap().cert
}

#[tokio::test]
async fn serves_info_over_https_with_the_printed_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = config(dir.path(), false);
    c.name = Some("Home server".into());
    let server = Server::start(&c).await.unwrap();
    let trusted = stored_cert(dir.path());

    let (fp, response) = get(server.local_addr(), &trusted, "/api/v1/info")
        .await
        .unwrap();
    assert_eq!(fp, server.fingerprint());
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let body = response.split("\r\n\r\n").nth(1).unwrap();
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["name"], "Home server");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    server.shutdown().await;
}

#[tokio::test]
async fn refuses_plain_http_and_unknown_paths() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(&config(dir.path(), false)).await.unwrap();
    let trusted = stored_cert(dir.path());

    let (_, response) = get(server.local_addr(), &trusted, "/nope").await.unwrap();
    assert!(response.starts_with("HTTP/1.1 404"), "{response}");

    // Plain HTTP gets no answer a client could use.
    let mut tcp = tokio::net::TcpStream::connect(server.local_addr())
        .await
        .unwrap();
    tcp.write_all(b"GET /api/v1/info HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    let _ = tcp.read_to_end(&mut buf).await;
    assert!(!String::from_utf8_lossy(&buf).contains("200 OK"));
    server.shutdown().await;
}

#[tokio::test]
async fn the_same_certificate_is_served_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let first = Server::start(&config(dir.path(), false)).await.unwrap();
    let trusted = stored_cert(dir.path());
    let (fp1, _) = get(first.local_addr(), &trusted, "/api/v1/info")
        .await
        .unwrap();
    first.shutdown().await;

    let second = Server::start(&config(dir.path(), false)).await.unwrap();
    let (fp2, _) = get(second.local_addr(), &trusted, "/api/v1/info")
        .await
        .unwrap();
    assert_eq!(fp1, fp2);
    second.shutdown().await;
}

#[tokio::test]
async fn ip_addresses_are_in_memory_only() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(&config(dir.path(), false)).await.unwrap();
    let trusted = stored_cert(dir.path());
    let tcp = tokio::net::TcpStream::connect(server.local_addr())
        .await
        .unwrap();
    let mut tls = connector(&trusted)
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .unwrap();

    // While the connection is open, the address is held in memory.
    let ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    for _ in 0..100 {
        if server.peers().connections_from(ip) == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(server.peers().connections_from(ip), 1);

    // It is not in the database file.
    let bytes = std::fs::read(dir.path().join(db::FILE_NAME)).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("127.0.0.1"));

    // And it is forgotten when the connection closes.
    let _ = tls.shutdown().await;
    drop(tls);
    for _ in 0..100 {
        if server.peers().distinct() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(server.peers().distinct(), 0);
    server.shutdown().await;
}

/// Run one request against a server on a current-thread runtime, so the
/// thread-local subscriber also sees the server's tasks. Returns the log.
fn logs_for_a_request(debug: bool) -> String {
    let log = Arc::new(Mutex::new(Vec::<u8>::new()));
    struct W(Arc<Mutex<Vec<u8>>>);
    impl io::Write for W {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let sink = log.clone();
    // The same filters main uses.
    let filter = if debug {
        "warn,hab_server=debug"
    } else {
        "warn,hab_server=info"
    };
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_ansi(false)
        .with_writer(move || W(sink.clone()))
        .finish();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tracing::subscriber::with_default(subscriber, || {
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let server = Server::start(&config(dir.path(), debug)).await.unwrap();
            let trusted = stored_cert(dir.path());
            get(server.local_addr(), &trusted, "/api/v1/info")
                .await
                .unwrap();
            // A failed handshake is logged too.
            let mut tcp = tokio::net::TcpStream::connect(server.local_addr())
                .await
                .unwrap();
            tcp.write_all(b"not tls\r\n\r\n").await.unwrap();
            let _ = tcp.read_to_end(&mut Vec::new()).await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            server.shutdown().await;
        })
    });
    let out = String::from_utf8(log.lock().unwrap().clone()).unwrap();
    out
}

#[test]
fn logs_hold_no_ip_addresses_unless_debug_logging_is_on() {
    let quiet = logs_for_a_request(false);
    assert!(!quiet.contains("127.0.0.1"), "{quiet}");

    let loud = logs_for_a_request(true);
    assert!(loud.contains("127.0.0.1"), "{loud}");
}

#[tokio::test]
async fn joining_needs_the_setup_code_before_anything_else() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(&config(dir.path(), false)).await.unwrap();
    let trusted = stored_cert(dir.path());
    let start = |code: &str, user: &str| {
        format!(r#"{{"setup_code":"{code}","username":"{user}","registration_request":"AAAA"}}"#)
    };

    // A wrong code gets nothing, whatever else is wrong with the request.
    let (status, body) = post(
        server.local_addr(),
        &trusted,
        "/api/v1/join/start",
        &start("nope", "chris"),
    )
    .await
    .unwrap();
    assert!(status.contains("403"), "{status}");
    assert!(body.contains("setup code"), "{body}");

    // The right code with a bad username or a garbled OPAQUE message is a 400.
    let code = server.setup_code();
    for (user, expected) in [("Not Allowed", "400"), ("chris", "400")] {
        let (status, _) = post(
            server.local_addr(),
            &trusted,
            "/api/v1/join/start",
            &start(&code, user),
        )
        .await
        .unwrap();
        assert!(status.contains(expected), "{user}: {status}");
    }

    // Finishing without a valid code or body creates no account.
    let (status, _) = post(server.local_addr(), &trusted, "/api/v1/join/finish", "{}")
        .await
        .unwrap();
    assert!(status.contains("422") || status.contains("400"), "{status}");
    assert!(server.check_setup_code(&code).unwrap());
    server.shutdown().await;
}
