//! What this device remembers about how it is set up: standalone, or joined
//! to a server. Secrets are not here; they go in the key store.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// The account this device joined, as shown in Settings → Account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub server_address: String,
    /// The pinned certificate, `AA:BB:..`.
    pub server_fingerprint: String,
    pub server_name: String,
    pub username: String,
    pub display_name: String,
    pub admin: bool,
    pub account_id: i64,
    pub device_id: i64,
    pub device_name: String,
    pub portable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Setup {
    /// No server. Everything works on this device.
    Standalone,
    Joined(Profile),
}

/// The `setup.json` file in the app's data folder. Missing means first start.
pub struct SetupFile {
    path: PathBuf,
}

impl SetupFile {
    pub fn in_dir(dir: &Path) -> SetupFile {
        SetupFile {
            path: dir.join("setup.json"),
        }
    }

    pub fn load(&self) -> io::Result<Option<Setup>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Written whole through a temporary file, readable by the user only.
    pub fn save(&self, setup: &Setup) -> io::Result<()> {
        crate::keystore::write_private(&self.path, &serde_json::to_vec_pretty(setup).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile {
            server_address: "home.lan:8443".into(),
            server_fingerprint: "AA:BB".into(),
            server_name: "Home".into(),
            username: "chris".into(),
            display_name: "Chris".into(),
            admin: true,
            account_id: 1,
            device_id: 1,
            device_name: "Desktop".into(),
            portable: false,
        }
    }

    #[test]
    fn missing_means_first_start_and_choices_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = SetupFile::in_dir(&dir.path().join("nested"));
        assert_eq!(file.load().unwrap(), None);
        file.save(&Setup::Standalone).unwrap();
        assert_eq!(file.load().unwrap(), Some(Setup::Standalone));
        file.save(&Setup::Joined(profile())).unwrap();
        assert_eq!(file.load().unwrap(), Some(Setup::Joined(profile())));
    }

    #[test]
    fn a_damaged_file_is_an_error_not_a_first_start() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("setup.json"), b"{").unwrap();
        assert!(SetupFile::in_dir(dir.path()).load().is_err());
    }
}
