//! A trusted certificate and key read from files, whatever issued them
//! (Tailscale for a `ts.net` name, certbot for a domain of your own), and
//! reloaded when the files change. Connections that ask for one of its names
//! get it; every other connection gets the self-signed certificate.

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

#[derive(Debug, thiserror::Error)]
pub enum TrustedError {
    #[error("cannot read {0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error("{0} holds no certificate")]
    NoCertificate(PathBuf),
    #[error("{0} holds no private key")]
    NoKey(PathBuf),
    #[error("the trusted certificate cannot be used: {0}")]
    Invalid(String),
}

/// What identifies one version of the two files.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp(Vec<(Option<SystemTime>, Option<u64>)>);

fn stamp(paths: &[&Path]) -> Stamp {
    Stamp(
        paths
            .iter()
            // `metadata` follows symlinks, which is how certbot lays out `live/`.
            .map(|p| {
                std::fs::metadata(p)
                    .map(|m| (m.modified().ok(), Some(m.len())))
                    .unwrap_or((None, None))
            })
            .collect(),
    )
}

fn read(cert: &Path, key: &Path) -> Result<Arc<CertifiedKey>, TrustedError> {
    let cert_pem = std::fs::read(cert).map_err(|e| TrustedError::Read(cert.into(), e))?;
    let key_pem = std::fs::read(key).map_err(|e| TrustedError::Read(key.into(), e))?;
    let chain: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_pem.as_slice())
        .collect::<Result<_, _>>()
        .map_err(|e| TrustedError::Invalid(e.to_string()))?;
    if chain.is_empty() {
        return Err(TrustedError::NoCertificate(cert.into()));
    }
    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())
        .map_err(|e| TrustedError::Invalid(e.to_string()))?
        .ok_or_else(|| TrustedError::NoKey(key.into()))?;
    let provider = rustls::crypto::ring::default_provider();
    // Also checks that the key belongs to the certificate.
    let certified = CertifiedKey::from_der(chain, key, &provider)
        .map_err(|e| TrustedError::Invalid(e.to_string()))?;
    Ok(Arc::new(certified))
}

/// Whether `cert` is valid for `name`, wildcards and all.
fn covers(cert: &CertificateDer<'_>, name: &str) -> bool {
    let (Ok(ee), Ok(server_name)) = (
        webpki::EndEntityCert::try_from(cert),
        ServerName::try_from(name),
    ) else {
        return false;
    };
    ee.verify_is_valid_for_subject_name(&server_name).is_ok()
}

/// Chooses the certificate for each connection.
#[derive(Debug)]
pub struct Resolver {
    self_signed: Arc<CertifiedKey>,
    trusted: RwLock<Option<Arc<CertifiedKey>>>,
}

impl Resolver {
    pub fn new(self_signed: Arc<CertifiedKey>) -> Resolver {
        Resolver {
            self_signed,
            trusted: RwLock::new(None),
        }
    }

    fn set_trusted(&self, key: Arc<CertifiedKey>) {
        *self.trusted.write().unwrap() = Some(key);
    }

    /// The certificate for a connection that asked for `sni`, if any.
    pub fn choose(&self, sni: Option<&str>) -> Arc<CertifiedKey> {
        if let (Some(name), Some(trusted)) = (sni, self.trusted.read().unwrap().as_ref()) {
            if trusted.cert.first().is_some_and(|c| covers(c, name)) {
                return trusted.clone();
            }
        }
        self.self_signed.clone()
    }

    /// SHA-256 fingerprint of the trusted certificate in use, if one is loaded.
    pub fn trusted_fingerprint(&self) -> Option<String> {
        let guard = self.trusted.read().unwrap();
        guard
            .as_ref()
            .and_then(|k| k.cert.first())
            .map(|c| crate::cert::fingerprint(c))
    }
}

impl ResolvesServerCert for Resolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.choose(hello.server_name()))
    }
}

/// The two files and what was last loaded from them.
pub struct Files {
    cert: PathBuf,
    key: PathBuf,
    seen: Stamp,
}

impl Files {
    /// Load the files into `resolver`. Failing here stops the server from
    /// starting, since the admin asked for a certificate that isn't usable.
    pub fn load(cert: &Path, key: &Path, resolver: &Resolver) -> Result<Files, TrustedError> {
        let seen = stamp(&[cert, key]);
        resolver.set_trusted(read(cert, key)?);
        Ok(Files {
            cert: cert.into(),
            key: key.into(),
            seen,
        })
    }

    /// Reload if either file changed. A bad new version is logged and the old
    /// one kept, so a renewal that is half written never takes the server down.
    /// Returns whether a new certificate is now in use.
    pub fn reload_if_changed(&mut self, resolver: &Resolver) -> bool {
        let now = stamp(&[&self.cert, &self.key]);
        if now == self.seen {
            return false;
        }
        match read(&self.cert, &self.key) {
            Ok(key) => {
                self.seen = now;
                resolver.set_trusted(key);
                tracing::info!("reloaded the trusted certificate");
                true
            }
            Err(e) => {
                // Remember it, so a bad file is reported once, not every poll;
                // the next change tries again.
                self.seen = now;
                tracing::warn!("keeping the previous trusted certificate: {e}");
                false
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair};

    pub struct Ca {
        pub cert: rcgen::Certificate,
        pub key: KeyPair,
    }

    pub fn ca() -> Ca {
        let mut params = CertificateParams::new(vec![]).unwrap();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, "Test CA");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let key = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        Ca { cert, key }
    }

    /// (certificate chain PEM, key PEM) issued by `ca` for `names`.
    pub fn issue(ca: &Ca, names: &[&str]) -> (String, String) {
        let params =
            CertificateParams::new(names.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                .unwrap();
        let key = KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &ca.cert, &ca.key).unwrap();
        (
            format!("{}{}", cert.pem(), ca.cert.pem()),
            key.serialize_pem(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::{cert, db::Db};

    fn self_signed() -> Arc<CertifiedKey> {
        let id = cert::load_or_create(&Db::in_memory().unwrap(), &[]).unwrap();
        let provider = rustls::crypto::ring::default_provider();
        Arc::new(CertifiedKey::from_der(vec![id.cert], id.key, &provider).unwrap())
    }

    #[test]
    fn serves_the_trusted_certificate_only_for_its_names() {
        let dir = tempfile::tempdir().unwrap();
        let (cert_pem, key_pem) = issue(&ca(), &["host.example.net", "*.lan.example.net"]);
        std::fs::write(dir.path().join("c.pem"), cert_pem).unwrap();
        std::fs::write(dir.path().join("k.pem"), key_pem).unwrap();
        let resolver = Resolver::new(self_signed());
        Files::load(
            &dir.path().join("c.pem"),
            &dir.path().join("k.pem"),
            &resolver,
        )
        .unwrap();

        let trusted = resolver.trusted.read().unwrap().clone().unwrap();
        let same = |a: &Arc<CertifiedKey>, b: &Arc<CertifiedKey>| Arc::ptr_eq(a, b);
        assert!(same(&resolver.choose(Some("host.example.net")), &trusted));
        assert!(same(&resolver.choose(Some("a.lan.example.net")), &trusted));
        for other in [Some("other.example.net"), Some("192.168.1.5"), None] {
            assert!(
                same(&resolver.choose(other), &resolver.self_signed),
                "{other:?}"
            );
        }
    }

    #[test]
    fn reloads_when_the_files_change_and_keeps_the_old_on_a_bad_write() {
        let dir = tempfile::tempdir().unwrap();
        let (c, k) = (dir.path().join("c.pem"), dir.path().join("k.pem"));
        let ca = ca();
        let (cert_pem, key_pem) = issue(&ca, &["host.example.net"]);
        std::fs::write(&c, &cert_pem).unwrap();
        std::fs::write(&k, &key_pem).unwrap();
        let resolver = Resolver::new(self_signed());
        let mut files = Files::load(&c, &k, &resolver).unwrap();
        let first = resolver.trusted_fingerprint().unwrap();
        assert!(!files.reload_if_changed(&resolver));

        // A renewal: new certificate and key (different length from a bad write).
        let (cert2, key2) = issue(&ca, &["host.example.net", "second.example.net"]);
        std::fs::write(&c, &cert2).unwrap();
        std::fs::write(&k, &key2).unwrap();
        assert!(files.reload_if_changed(&resolver));
        let second = resolver.trusted_fingerprint().unwrap();
        assert_ne!(first, second);
        assert!(Arc::ptr_eq(
            &resolver.choose(Some("second.example.net")),
            resolver.trusted.read().unwrap().as_ref().unwrap()
        ));

        // A half-written file: the old certificate stays.
        std::fs::write(&c, "-----BEGIN CERTIFICATE-----\nnope").unwrap();
        assert!(!files.reload_if_changed(&resolver));
        assert_eq!(resolver.trusted_fingerprint().unwrap(), second);

        // A certificate whose key is a different one is refused too.
        let (cert3, _) = issue(&ca, &["host.example.net"]);
        std::fs::write(&c, &cert3).unwrap();
        std::fs::write(&k, &key2).unwrap();
        assert!(!files.reload_if_changed(&resolver));
        assert_eq!(resolver.trusted_fingerprint().unwrap(), second);
    }

    #[test]
    fn refuses_unusable_files_at_start() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = Resolver::new(self_signed());
        let missing = dir.path().join("none.pem");
        assert!(Files::load(&missing, &missing, &resolver).is_err());
        std::fs::write(dir.path().join("e.pem"), "").unwrap();
        let e = dir.path().join("e.pem");
        assert!(matches!(
            Files::load(&e, &e, &resolver),
            Err(TrustedError::NoCertificate(_))
        ));
        assert!(resolver.trusted_fingerprint().is_none());
    }
}
