//! Where the account's secret keys are kept on this device: the Secret
//! Service (KWallet on Plasma) through `oo7`, or when none is running a file
//! only the user can read.

use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    #[error("the Secret Service refused: {0}")]
    SecretService(String),
    #[error("cannot use the key file: {0}")]
    File(#[from] io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// The user's identity and personal keys.
    Account,
    /// This device's own signing and sealing keys.
    Device,
}

impl KeyKind {
    fn as_str(self) -> &'static str {
        match self {
            KeyKind::Account => "account",
            KeyKind::Device => "device",
        }
    }
}

/// Which secret: one per server, user and kind.
#[derive(Debug, Clone)]
pub struct KeyId {
    pub server_fingerprint: String,
    pub username: String,
    pub kind: KeyKind,
}

impl KeyId {
    fn label(&self) -> String {
        format!("hab-bot {} key for {}", self.kind.as_str(), self.username)
    }

    fn file_name(&self) -> String {
        let digest = Sha256::digest(
            format!(
                "{}\0{}\0{}",
                self.server_fingerprint,
                self.username,
                self.kind.as_str()
            )
            .as_bytes(),
        );
        let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
        format!("{hex}.key")
    }
}

/// A folder of key files, each readable only by the user.
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: &Path) -> FileStore {
        FileStore {
            dir: dir.join("keys"),
        }
    }

    fn path(&self, id: &KeyId) -> PathBuf {
        self.dir.join(id.file_name())
    }
}

pub enum KeyStore {
    SecretService(oo7::Keyring),
    File(FileStore),
}

impl KeyStore {
    /// Use the Secret Service if one answers, and a key file in `data_dir`
    /// if not.
    pub async fn open(data_dir: &Path) -> KeyStore {
        let attempt = tokio::time::timeout(Duration::from_secs(5), async {
            let keyring = oo7::Keyring::new().await?;
            keyring.unlock().await?;
            Ok::<_, oo7::Error>(keyring)
        })
        .await;
        match attempt {
            Ok(Ok(keyring)) => KeyStore::SecretService(keyring),
            _ => KeyStore::File(FileStore::new(data_dir)),
        }
    }

    pub fn file(data_dir: &Path) -> KeyStore {
        KeyStore::File(FileStore::new(data_dir))
    }

    /// For Settings → This device.
    pub fn backend(&self) -> &'static str {
        match self {
            KeyStore::SecretService(_) => "Secret Service",
            KeyStore::File(_) => "a private file",
        }
    }

    fn attributes(id: &KeyId) -> HashMap<&'static str, String> {
        HashMap::from([
            ("application", "hab-bot".to_string()),
            ("kind", id.kind.as_str().to_string()),
            ("server", id.server_fingerprint.clone()),
            ("username", id.username.clone()),
        ])
    }

    pub async fn put(&self, id: &KeyId, secret: &[u8]) -> Result<(), KeyStoreError> {
        match self {
            KeyStore::SecretService(keyring) => {
                let attrs = Self::attributes(id);
                let attrs: HashMap<&str, &str> =
                    attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
                keyring
                    .create_item(&id.label(), &attrs, oo7::Secret::blob(secret), true)
                    .await
                    .map_err(|e| KeyStoreError::SecretService(e.to_string()))?;
                Ok(())
            }
            KeyStore::File(f) => {
                write_private(&f.path(id), STANDARD.encode(secret).as_bytes())?;
                Ok(())
            }
        }
    }

    pub async fn get(&self, id: &KeyId) -> Result<Option<Vec<u8>>, KeyStoreError> {
        match self {
            KeyStore::SecretService(keyring) => {
                let attrs = Self::attributes(id);
                let attrs: HashMap<&str, &str> =
                    attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let items = keyring
                    .search_items(&attrs)
                    .await
                    .map_err(|e| KeyStoreError::SecretService(e.to_string()))?;
                match items.first() {
                    Some(item) => Ok(Some(
                        item.secret()
                            .await
                            .map_err(|e| KeyStoreError::SecretService(e.to_string()))?
                            .as_bytes()
                            .to_vec(),
                    )),
                    None => Ok(None),
                }
            }
            KeyStore::File(f) => match std::fs::read_to_string(f.path(id)) {
                Ok(text) => STANDARD
                    .decode(text.trim())
                    .map(Some)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e).into()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            },
        }
    }

    pub async fn delete(&self, id: &KeyId) -> Result<(), KeyStoreError> {
        match self {
            KeyStore::SecretService(keyring) => {
                let attrs = Self::attributes(id);
                let attrs: HashMap<&str, &str> =
                    attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
                keyring
                    .delete(&attrs)
                    .await
                    .map_err(|e| KeyStoreError::SecretService(e.to_string()))
            }
            KeyStore::File(f) => match std::fs::remove_file(f.path(id)) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.into()),
            },
        }
    }
}

/// Write `bytes` to `path` through a temporary file that only the user can
/// read, creating the folder the same way.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(kind: KeyKind) -> KeyId {
        KeyId {
            server_fingerprint: "AA:BB".into(),
            username: "chris".into(),
            kind,
        }
    }

    #[tokio::test]
    async fn the_file_store_keeps_deletes_and_separates_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::file(dir.path());
        assert_eq!(store.backend(), "a private file");
        assert_eq!(store.get(&id(KeyKind::Account)).await.unwrap(), None);
        store.put(&id(KeyKind::Account), b"one").await.unwrap();
        store.put(&id(KeyKind::Device), b"two").await.unwrap();
        assert_eq!(
            store.get(&id(KeyKind::Account)).await.unwrap().unwrap(),
            b"one"
        );
        store.put(&id(KeyKind::Account), b"three").await.unwrap();
        assert_eq!(
            store.get(&id(KeyKind::Account)).await.unwrap().unwrap(),
            b"three"
        );
        store.delete(&id(KeyKind::Account)).await.unwrap();
        store.delete(&id(KeyKind::Account)).await.unwrap();
        assert_eq!(store.get(&id(KeyKind::Account)).await.unwrap(), None);
        assert_eq!(
            store.get(&id(KeyKind::Device)).await.unwrap().unwrap(),
            b"two"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn key_files_and_their_folder_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::file(dir.path());
        store.put(&id(KeyKind::Account), b"secret").await.unwrap();
        let keys = dir.path().join("keys");
        assert_eq!(
            std::fs::metadata(&keys).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for entry in std::fs::read_dir(&keys).unwrap() {
            let mode = entry.unwrap().metadata().unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn with_no_secret_service_the_default_is_a_file() {
        // No session bus in a test run, so opening falls back to the file.
        let dir = tempfile::tempdir().unwrap();
        let saved = std::env::var_os("DBUS_SESSION_BUS_ADDRESS");
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/bus");
        let store = KeyStore::open(dir.path()).await;
        match saved {
            Some(v) => std::env::set_var("DBUS_SESSION_BUS_ADDRESS", v),
            None => std::env::remove_var("DBUS_SESSION_BUS_ADDRESS"),
        }
        assert_eq!(store.backend(), "a private file");
    }
}
