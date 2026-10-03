//! The self-signed certificate: made on first start, kept in the database
//! across restarts, and identified by its SHA-256 fingerprint.

use crate::db::{Db, DbError};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

const CERT_KEY: &str = "tls_cert_pem";
const PRIVATE_KEY: &str = "tls_key_pem";

#[derive(Debug, thiserror::Error)]
pub enum CertError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("certificate error: {0}")]
    Generate(#[from] rcgen::Error),
    #[error("stored certificate is unreadable: {0}")]
    Stored(String),
}

pub struct Identity {
    pub cert: CertificateDer<'static>,
    pub key: PrivateKeyDer<'static>,
    /// SHA-256 of the DER certificate, as upper-case hex pairs joined by colons.
    pub fingerprint: String,
    pub created: bool,
}

/// `AA:BB:...` SHA-256 of a DER certificate.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Load the stored certificate, or make and store one for `names`.
pub fn load_or_create(db: &Db, names: &[String]) -> Result<Identity, CertError> {
    if let (Some(cert_pem), Some(key_pem)) = (db.get(CERT_KEY)?, db.get(PRIVATE_KEY)?) {
        return parse(&cert_pem, &key_pem, false);
    }
    let mut sans: Vec<String> = vec!["localhost".into()];
    sans.extend(names.iter().cloned());
    let mut params = CertificateParams::new(sans)?;
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, "hab-bot server");
    // Devices pin the fingerprint, so a long life avoids a re-pin.
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    params.not_after = rcgen::date_time_ymd(2124, 1, 1);
    let key = KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    let (cert_pem, key_pem) = (cert.pem(), key.serialize_pem());
    db.set(CERT_KEY, &cert_pem)?;
    db.set(PRIVATE_KEY, &key_pem)?;
    parse(&cert_pem, &key_pem, true)
}

fn parse(cert_pem: &str, key_pem: &str, created: bool) -> Result<Identity, CertError> {
    let cert = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .next()
        .ok_or_else(|| CertError::Stored("no certificate".into()))?
        .map_err(|e| CertError::Stored(e.to_string()))?;
    let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())
        .map_err(|e| CertError::Stored(e.to_string()))?
        .ok_or_else(|| CertError::Stored("no private key".into()))?;
    Ok(Identity {
        fingerprint: fingerprint(&cert),
        cert,
        key,
        created,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_made_once_and_kept() {
        let db = Db::in_memory().unwrap();
        let first = load_or_create(&db, &[]).unwrap();
        assert!(first.created);
        let second = load_or_create(&db, &[]).unwrap();
        assert!(!second.created);
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(first.cert, second.cert);
    }

    #[test]
    fn fingerprint_is_colon_separated_sha256() {
        let fp = fingerprint(b"abc");
        assert_eq!(fp.split(':').count(), 32);
        assert!(fp.starts_with("BA:78:16:BF"));
    }

    #[test]
    fn separate_databases_get_different_certificates() {
        let a = load_or_create(&Db::in_memory().unwrap(), &[]).unwrap();
        let b = load_or_create(&Db::in_memory().unwrap(), &[]).unwrap();
        assert_ne!(a.fingerprint, b.fingerprint);
    }
}
