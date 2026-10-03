//! The server's one SQLite file.

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "hab-server.db";

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("cannot use data folder {0}: {1}")]
    Folder(PathBuf, std::io::Error),
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open (creating if needed) the database in `dir`.
    pub fn open(dir: &Path) -> Result<Db, DbError> {
        std::fs::create_dir_all(dir).map_err(|e| DbError::Folder(dir.to_path_buf(), e))?;
        let path = dir.join(FILE_NAME);
        let conn = Connection::open(&path)?;
        #[cfg(unix)]
        {
            // The file holds the certificate's private key.
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| DbError::Folder(dir.to_path_buf(), e))?;
        }
        Db::init(conn)
    }

    pub fn in_memory() -> Result<Db, DbError> {
        Db::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db, DbError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            // Before joining existed, `accounts` was a placeholder that only
            // held a username. It never held a real account.
            conn.execute_batch("DROP TABLE IF EXISTS accounts;")?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS accounts (
                 id              INTEGER PRIMARY KEY,
                 username        TEXT NOT NULL UNIQUE,
                 display_name    TEXT NOT NULL,
                 admin           INTEGER NOT NULL,
                 created_at      INTEGER NOT NULL,
                 identity_alg    TEXT NOT NULL,
                 identity_public BLOB NOT NULL,
                 -- OPAQUE's registration record: all the server learns of the password.
                 opaque_record   BLOB NOT NULL,
                 -- The Argon2id cost the device used, so it can be raised later.
                 kdf_alg         TEXT NOT NULL,
                 kdf_memory_kib  INTEGER NOT NULL,
                 kdf_passes      INTEGER NOT NULL,
                 kdf_lanes       INTEGER NOT NULL,
                 -- The identity and personal keys, encrypted on the device.
                 bundle_alg      TEXT NOT NULL,
                 bundle_nonce    BLOB NOT NULL,
                 bundle          BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS devices (
                 id             INTEGER PRIMARY KEY,
                 account_id     INTEGER NOT NULL REFERENCES accounts(id),
                 name           TEXT NOT NULL,
                 portable       INTEGER NOT NULL,
                 alg            TEXT NOT NULL,
                 signing_public BLOB NOT NULL,
                 sealing_public BLOB NOT NULL,
                 -- The identity key's signature over both public keys.
                 signature      BLOB NOT NULL,
                 created_at     INTEGER NOT NULL
             );
             PRAGMA user_version = 1;",
        )?;
        Ok(Db { conn })
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, DbError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn account_exists(&self) -> Result<bool, DbError> {
        Ok(self
            .conn
            .query_row("SELECT EXISTS(SELECT 1 FROM accounts)", [], |r| r.get(0))?)
    }

    /// Create the first account and its first device, as the server admin,
    /// in one transaction. `allowed` is told whether an account already
    /// exists, read inside the transaction, and says whether the setup code
    /// still works. Because the check and the insert share a write lock, two
    /// joins can't both succeed.
    pub fn create_first_account(
        &self,
        allowed: impl FnOnce(bool) -> bool,
        new: &NewAccount,
        now: i64,
    ) -> Result<Option<Created>, DbError> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let exists: bool =
            tx.query_row("SELECT EXISTS(SELECT 1 FROM accounts)", [], |r| r.get(0))?;
        if !allowed(exists) || exists {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO accounts (username, display_name, admin, created_at, identity_alg,
                 identity_public, opaque_record, kdf_alg, kdf_memory_kib, kdf_passes, kdf_lanes,
                 bundle_alg, bundle_nonce, bundle)
             VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                new.username,
                new.display_name,
                now,
                new.identity_alg,
                new.identity_public,
                new.opaque_record,
                new.kdf_alg,
                new.kdf_memory_kib,
                new.kdf_passes,
                new.kdf_lanes,
                new.bundle_alg,
                new.bundle_nonce,
                new.bundle,
            ],
        )?;
        let account_id = tx.last_insert_rowid();
        let d = &new.device;
        tx.execute(
            "INSERT INTO devices (account_id, name, portable, alg, signing_public,
                 sealing_public, signature, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                account_id,
                d.name,
                d.portable,
                d.alg,
                d.signing_public,
                d.sealing_public,
                d.signature,
                now
            ],
        )?;
        let device_id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(Some(Created {
            account_id,
            device_id,
        }))
    }

    /// The bundle and key-derivation cost stored for a username.
    #[cfg(test)]
    pub fn account(&self, username: &str) -> Result<Option<StoredAccount>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT display_name, admin, bundle, opaque_record,
                        (SELECT COUNT(*) FROM devices WHERE account_id = accounts.id)
                 FROM accounts WHERE username = ?1",
                [username],
                |r| {
                    Ok(StoredAccount {
                        display_name: r.get(0)?,
                        admin: r.get(1)?,
                        bundle: r.get(2)?,
                        opaque_record: r.get(3)?,
                        devices: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }
}

/// What joining stores for an account and its first device.
pub struct NewAccount {
    pub username: String,
    pub display_name: String,
    pub identity_alg: String,
    pub identity_public: Vec<u8>,
    pub opaque_record: Vec<u8>,
    pub kdf_alg: String,
    pub kdf_memory_kib: u32,
    pub kdf_passes: u32,
    pub kdf_lanes: u32,
    pub bundle_alg: String,
    pub bundle_nonce: Vec<u8>,
    pub bundle: Vec<u8>,
    pub device: NewDevice,
}

pub struct NewDevice {
    pub name: String,
    pub portable: bool,
    pub alg: String,
    pub signing_public: Vec<u8>,
    pub sealing_public: Vec<u8>,
    pub signature: Vec<u8>,
}

pub struct Created {
    pub account_id: i64,
    pub device_id: i64,
}

#[cfg(test)]
pub struct StoredAccount {
    pub display_name: String,
    pub admin: bool,
    pub bundle: Vec<u8>,
    pub opaque_record: Vec<u8>,
    pub devices: i64,
}

#[cfg(test)]
pub(crate) fn test_account(username: &str) -> NewAccount {
    NewAccount {
        username: username.into(),
        display_name: "Chris".into(),
        identity_alg: "ed25519/v1".into(),
        identity_public: vec![1; 32],
        opaque_record: vec![2; 8],
        kdf_alg: "argon2id".into(),
        kdf_memory_kib: 65536,
        kdf_passes: 3,
        kdf_lanes: 1,
        bundle_alg: "x".into(),
        bundle_nonce: vec![3; 24],
        bundle: vec![4; 80],
        device: NewDevice {
            name: "Desktop".into(),
            portable: false,
            alg: "d".into(),
            signing_public: vec![5; 32],
            sealing_public: vec![6; 32],
            signature: vec![7; 64],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_overwrite() {
        let db = Db::in_memory().unwrap();
        assert_eq!(db.get("a").unwrap(), None);
        db.set("a", "1").unwrap();
        db.set("a", "2").unwrap();
        assert_eq!(db.get("a").unwrap().as_deref(), Some("2"));
    }

    #[test]
    fn knows_when_an_account_exists() {
        let db = Db::in_memory().unwrap();
        assert!(!db.account_exists().unwrap());
        db.create_first_account(|_| true, &test_account("chris"), 0)
            .unwrap()
            .unwrap();
        assert!(db.account_exists().unwrap());
    }

    #[test]
    fn the_first_account_is_the_admin_and_the_only_one() {
        let db = Db::in_memory().unwrap();
        let made = db
            .create_first_account(|exists| !exists, &test_account("chris"), 5)
            .unwrap()
            .unwrap();
        assert_eq!((made.account_id, made.device_id), (1, 1));
        let stored = db.account("chris").unwrap().unwrap();
        assert!(stored.admin);
        assert_eq!(stored.devices, 1);
        assert_eq!(stored.bundle, vec![4; 80]);
        // Even a caller that says yes can't add a second one.
        assert!(db
            .create_first_account(|_| true, &test_account("other"), 6)
            .unwrap()
            .is_none());
        assert!(db.account("other").unwrap().is_none());
    }

    #[test]
    fn a_refused_join_stores_nothing() {
        let db = Db::in_memory().unwrap();
        assert!(db
            .create_first_account(|_| false, &test_account("chris"), 0)
            .unwrap()
            .is_none());
        assert!(!db.account_exists().unwrap());
    }

    #[test]
    fn a_failed_device_insert_rolls_the_account_back() {
        let db = Db::in_memory().unwrap();
        // Break the second insert by dropping the table it needs.
        db.conn.execute_batch("DROP TABLE devices").unwrap();
        assert!(db
            .create_first_account(|_| true, &test_account("chris"), 0)
            .is_err());
        assert!(!db.account_exists().unwrap());
    }

    #[test]
    fn the_placeholder_accounts_table_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join(FILE_NAME)).unwrap();
            conn.execute_batch(
                "CREATE TABLE accounts (id INTEGER PRIMARY KEY, username TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL);",
            )
            .unwrap();
        }
        let db = Db::open(dir.path()).unwrap();
        db.create_first_account(|_| true, &test_account("chris"), 0)
            .unwrap()
            .unwrap();
    }

    #[test]
    fn keeps_state_in_one_file_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b");
        Db::open(&nested).unwrap().set("k", "v").unwrap();
        assert_eq!(
            Db::open(&nested).unwrap().get("k").unwrap().as_deref(),
            Some("v")
        );
        assert!(nested.join(FILE_NAME).exists());
    }
}
