//! HTTPS to a server we trust by certificate fingerprint, not by name.
//!
//! A home server's self-signed certificate is checked by its SHA-256
//! fingerprint (`AA:BB:..`). The name in it is not checked, because the same
//! server is reached by IP address, LAN name or VPN name. Signatures in the
//! handshake are still verified, so only the holder of the key can use it.
//!
//! A server may also present a trusted certificate (one a public authority
//! issued, such as Let's Encrypt or Tailscale's) when the app reaches it by a
//! name that certificate covers. That is accepted too, when it chains to a
//! public root and is valid for the name used.

use hab_proto::{auth, DeviceKeys};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::rt::TokioIo;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::{ring, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

pub const DEFAULT_PORT: u16 = 443;
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE: usize = 16 << 20;

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("the server address is not valid")]
    Address,
    #[error("cannot reach the server: {0}")]
    Connect(String),
    #[error("the server's certificate is not the one that was confirmed")]
    WrongCertificate,
    #[error("the server answered badly: {0}")]
    Protocol(String),
    #[error("the server said no ({status}): {message}")]
    Status { status: u16, message: String },
}

/// `AA:BB:..` SHA-256 of a DER certificate. The same format the server prints.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Compare fingerprints however the user typed them.
pub fn same_fingerprint(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_hexdigit())
            .map(|c| c.to_ascii_uppercase())
            .collect::<String>()
    };
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && a == b
}

#[derive(Debug)]
struct PinVerifier {
    /// None accepts any certificate and only records it.
    expected: Option<String>,
    /// Accepts a certificate from a trusted authority for the name used, when
    /// it is not the pinned one.
    trusted: Arc<WebPkiServerVerifier>,
    seen: Mutex<Option<String>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint(end_entity);
        *self.seen.lock().unwrap() = Some(fp.clone());
        match &self.expected {
            Some(expected) if !same_fingerprint(expected, &fp) => self
                .trusted
                .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
                .map_err(|_| {
                    rustls::Error::InvalidCertificate(
                        rustls::CertificateError::ApplicationVerificationFailure,
                    )
                }),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Split `address` into the host and port, accepting `https://`, a missing
/// port (443) and a trailing slash.
pub fn parse_address(address: &str) -> Result<(String, u16), TlsError> {
    let a = address.trim();
    let a = a
        .strip_prefix("https://")
        .unwrap_or(a)
        .trim_end_matches('/');
    if a.is_empty() || a.contains('/') || a.contains(char::is_whitespace) {
        return Err(TlsError::Address);
    }
    // [v6]:port, [v6], host:port, host
    let (host, port) = if let Some(rest) = a.strip_prefix('[') {
        let (h, after) = rest.split_once(']').ok_or(TlsError::Address)?;
        match after.strip_prefix(':') {
            Some(p) => (h, Some(p)),
            None if after.is_empty() => (h, None),
            None => return Err(TlsError::Address),
        }
    } else if a.matches(':').count() == 1 {
        let (h, p) = a.split_once(':').unwrap();
        (h, Some(p))
    } else {
        // A name, an IPv4 address, or a bare IPv6 address (which has no port).
        (a, None)
    };
    let port = match port {
        Some(p) => p.parse().map_err(|_| TlsError::Address)?,
        None => DEFAULT_PORT,
    };
    if host.is_empty() {
        return Err(TlsError::Address);
    }
    Ok((host.to_string(), port))
}

/// The public authorities a trusted server certificate may chain to.
fn public_roots() -> rustls::RootCertStore {
    rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    }
}

pub(crate) async fn connect(
    address: &str,
    expected: Option<&str>,
) -> Result<(tokio_rustls::client::TlsStream<TcpStream>, Option<String>), TlsError> {
    connect_with_roots(address, expected, public_roots()).await
}

pub(crate) async fn connect_with_roots(
    address: &str,
    expected: Option<&str>,
    roots: rustls::RootCertStore,
) -> Result<(tokio_rustls::client::TlsStream<TcpStream>, Option<String>), TlsError> {
    let (host, port) = parse_address(address)?;
    let provider = Arc::new(ring::default_provider());
    let trusted = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(|e| TlsError::Connect(e.to_string()))?;
    let verifier = Arc::new(PinVerifier {
        expected: expected.map(str::to_string),
        trusted,
        seen: Mutex::new(None),
        provider: provider.clone(),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Connect(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    let name = ServerName::try_from(host.clone()).map_err(|_| TlsError::Address)?;
    let tcp = tokio::time::timeout(TIMEOUT, TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| TlsError::Connect("timed out".into()))?
        .map_err(|e| TlsError::Connect(e.to_string()))?;
    let tls = tokio::time::timeout(
        TIMEOUT,
        TlsConnector::from(Arc::new(config)).connect(name, tcp),
    )
    .await
    .map_err(|_| TlsError::Connect("timed out".into()))?
    .map_err(|e| {
        if expected.is_some() && verifier.seen.lock().unwrap().is_some() {
            TlsError::WrongCertificate
        } else {
            TlsError::Connect(e.to_string())
        }
    })?;
    let seen = verifier.seen.lock().unwrap().clone();
    Ok((tls, seen))
}

/// Connect without trusting anything, and report the certificate the server
/// presents. Nothing is sent. The caller shows the fingerprint to the user,
/// and only what they confirm is pinned.
pub async fn probe(address: &str) -> Result<String, TlsError> {
    let (_tls, seen) = connect(address, None).await?;
    seen.ok_or_else(|| TlsError::Protocol("no certificate".into()))
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The headers that sign a request as `device_id`.
pub(crate) fn signed_headers(
    device: &DeviceKeys,
    device_id: i64,
    method: &str,
    path: &str,
    body: &[u8],
) -> Vec<(&'static str, String)> {
    let time = unix_now();
    vec![
        (auth::HEADER_DEVICE, device_id.to_string()),
        (auth::HEADER_TIME, time.to_string()),
        (
            auth::HEADER_SIGNATURE,
            auth::sign_request(device, device_id, method, path, time, body),
        ),
    ]
}

/// A server whose certificate is pinned.
#[derive(Debug, Clone)]
pub struct Pinned {
    pub address: String,
    pub fingerprint: String,
}

impl Pinned {
    pub fn new(address: &str, fingerprint: &str) -> Pinned {
        Pinned {
            address: address.into(),
            fingerprint: fingerprint.into(),
        }
    }

    /// `GET /api/v1/info`: the name and version the user confirms.
    pub async fn info(&self) -> Result<ServerInfo, TlsError> {
        self.request::<(), _>("GET", "/api/v1/info", None).await
    }

    pub async fn post<B: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R, TlsError> {
        self.request("POST", path, Some(body)).await
    }

    /// POST JSON signed by a device, which is how the server knows who is
    /// asking (see `hab_proto::auth`).
    pub async fn signed_post<B: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
        device_id: i64,
        device: &DeviceKeys,
    ) -> Result<R, TlsError> {
        let payload = serde_json::to_vec(body).map_err(|e| TlsError::Protocol(e.to_string()))?;
        let headers = signed_headers(device, device_id, "POST", path, &payload);
        self.request_with("POST", path, Some(payload), &headers)
            .await
    }

    async fn request<B: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&B>,
    ) -> Result<R, TlsError> {
        let payload = match body {
            Some(b) => Some(serde_json::to_vec(b).map_err(|e| TlsError::Protocol(e.to_string()))?),
            None => None,
        };
        self.request_with(method, path, payload, &[]).await
    }

    async fn request_with<R: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
        headers: &[(&'static str, String)],
    ) -> Result<R, TlsError> {
        let (tls, _) = connect(&self.address, Some(&self.fingerprint)).await?;
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .map_err(|e| TlsError::Protocol(e.to_string()))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let (host, port) = parse_address(&self.address)?;
        let has_body = body.is_some();
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", format!("{host}:{port}"))
            .header("accept", "application/json");
        if has_body {
            req = req.header("content-type", "application/json");
        }
        for (name, value) in headers {
            req = req.header(*name, value);
        }
        let req = req
            .body(Full::new(Bytes::from(body.unwrap_or_default())))
            .map_err(|e| TlsError::Protocol(e.to_string()))?;
        let response = tokio::time::timeout(TIMEOUT, sender.send_request(req))
            .await
            .map_err(|_| TlsError::Connect("timed out".into()))?
            .map_err(|e| TlsError::Protocol(e.to_string()))?;
        let status = response.status().as_u16();
        let bytes = Limited::new(response.into_body(), MAX_RESPONSE)
            .collect()
            .await
            .map_err(|e| TlsError::Protocol(e.to_string()))?
            .to_bytes();
        if !(200..300).contains(&status) {
            let message = serde_json::from_slice::<hab_proto::wire::ErrorBody>(&bytes)
                .map(|e| e.error)
                .unwrap_or_default();
            return Err(TlsError::Status { status, message });
        }
        serde_json::from_slice(&bytes).map_err(|e| TlsError::Protocol(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse_address("home.lan").unwrap(), ("home.lan".into(), 443));
        assert_eq!(
            parse_address("https://home.lan:8443/").unwrap(),
            ("home.lan".into(), 8443)
        );
        assert_eq!(
            parse_address("192.168.1.5:9").unwrap(),
            ("192.168.1.5".into(), 9)
        );
        assert_eq!(parse_address("[::1]:7").unwrap(), ("::1".into(), 7));
        assert_eq!(parse_address("[::1]").unwrap(), ("::1".into(), 443));
        assert!(parse_address("").is_err());
        assert!(parse_address("a b").is_err());
        assert!(parse_address("a/b").is_err());
        assert!(parse_address("host:notaport").is_err());
        assert!(parse_address(":80").is_err());
    }

    #[test]
    fn fingerprints_compare_loosely() {
        assert!(same_fingerprint("AA:bb:01", "aabb01"));
        assert!(!same_fingerprint("AA:BB", "AA:BC"));
        assert!(!same_fingerprint("", ""));
    }

    /// A CA, and a server that serves a certificate it issued for `localhost`
    /// beside its own self-signed one.
    async fn server_with_trusted_cert(
        dir: &std::path::Path,
    ) -> (hab_server::Server, rustls::RootCertStore) {
        use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
        let mut ca_params = CertificateParams::new(vec![]).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let key = KeyPair::generate().unwrap();
        let leaf = CertificateParams::new(vec!["localhost".to_string()])
            .unwrap()
            .signed_by(&key, &ca, &ca_key)
            .unwrap();
        std::fs::write(dir.join("c.pem"), leaf.pem()).unwrap();
        std::fs::write(dir.join("k.pem"), key.serialize_pem()).unwrap();
        let server = hab_server::Server::start(&hab_server::Config {
            data_dir: dir.join("data"),
            listen: "127.0.0.1:0".parse().unwrap(),
            tls_cert: Some(dir.join("c.pem")),
            tls_key: Some(dir.join("k.pem")),
            ..hab_server::Config::default()
        })
        .await
        .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(ca.der().clone()).unwrap();
        (server, roots)
    }

    #[tokio::test]
    async fn accepts_the_pinned_certificate_and_a_trusted_one() {
        let dir = tempfile::tempdir().unwrap();
        let (server, roots) = server_with_trusted_cert(dir.path()).await;
        let port = server.local_addr().port();
        let pinned = server.fingerprint().to_string();
        let trusted = server.trusted_fingerprint().unwrap();
        assert_ne!(pinned, trusted);

        // By a name the trusted certificate covers, the server presents it,
        // and the app accepts it though it isn't the pinned one.
        let by_name = format!("localhost:{port}");
        let (_, seen) = connect_with_roots(&by_name, Some(&pinned), roots.clone())
            .await
            .unwrap();
        assert_eq!(seen.as_deref(), Some(trusted.as_str()));

        // By IP address, the server presents the self-signed one: pinned.
        let by_ip = format!("127.0.0.1:{port}");
        let (_, seen) = connect_with_roots(&by_ip, Some(&pinned), roots.clone())
            .await
            .unwrap();
        assert_eq!(seen.as_deref(), Some(pinned.as_str()));

        // Neither pinned nor chaining to a root the app trusts: refused.
        let mut other = rcgen::CertificateParams::new(vec![]).unwrap();
        other.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let other = other
            .self_signed(&rcgen::KeyPair::generate().unwrap())
            .unwrap();
        let mut nothing = rustls::RootCertStore::empty();
        nothing.add(other.der().clone()).unwrap();
        let err = connect_with_roots(&by_name, Some("00:11"), nothing)
            .await
            .unwrap_err();
        assert!(matches!(err, TlsError::WrongCertificate), "{err}");
        // A self-signed certificate that isn't the pinned one is refused too.
        let err = connect_with_roots(&by_ip, Some("00:11"), roots)
            .await
            .unwrap_err();
        assert!(matches!(err, TlsError::WrongCertificate), "{err}");
        server.shutdown().await;
    }
}
