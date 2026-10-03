//! Running the server: the trusted certificate beside the self-signed one,
//! reloading it, and the backup while the server keeps answering.

use hab_server::{backup, cert, db, Config, Server};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsConnector;

/// Accepts any certificate, so a test can see which one the server presents.
#[derive(Debug)]
struct AcceptAny;

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The fingerprint of the certificate presented to a client that asks for
/// `name`, and the body of `/api/v1/info`.
async fn presented(addr: SocketAddr, name: ServerName<'static>) -> (String, String) {
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(AcceptAny))
    .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut tls = TlsConnector::from(Arc::new(cfg))
        .connect(name, tcp)
        .await
        .unwrap();
    let fp = cert::fingerprint(&tls.get_ref().1.peer_certificates().unwrap()[0]);
    tls.write_all(b"GET /api/v1/info HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = String::new();
    let _ = tls.read_to_string(&mut body).await;
    (fp, body)
}

/// Write a certificate for `names` and its key, issued by a fresh CA.
fn write_certificate(dir: &Path, names: &[&str]) {
    let mut ca_params = CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(names.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .unwrap()
        .signed_by(&key, &ca, &ca_key)
        .unwrap();
    // Write the key first: the server reloads when either file changes.
    std::fs::write(dir.join("key.pem"), key.serialize_pem()).unwrap();
    std::fs::write(dir.join("cert.pem"), format!("{}{}", leaf.pem(), ca.pem())).unwrap();
}

fn config(data: &Path, certs: &Path) -> Config {
    Config {
        data_dir: data.to_path_buf(),
        listen: "127.0.0.1:0".parse().unwrap(),
        tls_cert: Some(certs.join("cert.pem")),
        tls_key: Some(certs.join("key.pem")),
        tls_poll: Duration::from_millis(50),
        ..Config::default()
    }
}

fn dns(name: &str) -> ServerName<'static> {
    ServerName::try_from(name.to_string()).unwrap()
}

#[tokio::test]
async fn the_trusted_certificate_is_for_its_names_and_the_self_signed_one_for_the_rest() {
    let (data, certs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write_certificate(certs.path(), &["home.example.net"]);
    let server = Server::start(&config(data.path(), certs.path()))
        .await
        .unwrap();
    let trusted = server.trusted_fingerprint().unwrap();
    assert_ne!(trusted, server.fingerprint());

    let (fp, body) = presented(server.local_addr(), dns("home.example.net")).await;
    assert_eq!(fp, trusted);
    assert!(body.contains("200 OK"), "{body}");
    // Another name, an IP address (which sends no name) and a name the
    // trusted certificate doesn't cover all get the self-signed one.
    for name in [
        dns("other.example.net"),
        ServerName::try_from("127.0.0.1").unwrap(),
        dns("localhost"),
    ] {
        let (fp, body) = presented(server.local_addr(), name.clone()).await;
        assert_eq!(fp, server.fingerprint(), "{name:?}");
        assert!(body.contains("200 OK"), "{body}");
    }
    server.shutdown().await;
}

#[tokio::test]
async fn a_renewed_certificate_is_served_without_a_restart() {
    let (data, certs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write_certificate(certs.path(), &["home.example.net"]);
    let server = Server::start(&config(data.path(), certs.path()))
        .await
        .unwrap();
    let before = server.trusted_fingerprint().unwrap();

    // A renewal that also adds a name, so the files differ in size.
    write_certificate(certs.path(), &["home.example.net", "new.example.net"]);
    let mut after = before.clone();
    for _ in 0..100 {
        after = server.trusted_fingerprint().unwrap();
        if after != before {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_ne!(after, before, "the new certificate was never loaded");
    let (fp, _) = presented(server.local_addr(), dns("new.example.net")).await;
    assert_eq!(fp, after);

    // A broken write leaves the working certificate in place.
    std::fs::write(certs.path().join("cert.pem"), "garbage").unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (fp, body) = presented(server.local_addr(), dns("new.example.net")).await;
    assert_eq!(fp, after);
    assert!(body.contains("200 OK"));
    server.shutdown().await;
}

#[tokio::test]
async fn unusable_certificate_files_stop_the_server_from_starting() {
    let (data, certs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    // Nothing written in `certs`.
    assert!(Server::start(&config(data.path(), certs.path()))
        .await
        .is_err());
}

#[tokio::test]
async fn backs_up_while_the_server_keeps_answering() {
    let (data, backups) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let folder = backups.path().join("nested");
    let server = Server::start(&Config {
        data_dir: data.path().to_path_buf(),
        listen: "127.0.0.1:0".parse().unwrap(),
        name: Some("Home server".into()),
        backup: Some(backup::BackupConfig {
            dir: folder.clone(),
            at: backup::TimeOfDay::DEFAULT,
            keep: 3,
        }),
        ..Config::default()
    })
    .await
    .unwrap();

    // Requests keep being answered while a backup runs.
    let addr = server.local_addr();
    let busy = tokio::spawn(async move {
        for _ in 0..20 {
            let (_, body) = presented(addr, dns("localhost")).await;
            assert!(body.contains("Home server"), "{body}");
        }
    });
    let file = server.backup_now().await.unwrap();
    busy.await.unwrap();
    let (_, body) = presented(addr, dns("localhost")).await;
    assert!(body.contains("200 OK"));
    server.shutdown().await;

    // Restoring is putting the file back as the database.
    let restored = tempfile::tempdir().unwrap();
    std::fs::copy(&file, restored.path().join(db::FILE_NAME)).unwrap();
    let again = Server::start(&Config {
        data_dir: restored.path().to_path_buf(),
        listen: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    })
    .await
    .unwrap();
    assert_eq!(again.name(), "Home server");
    again.shutdown().await;
}
