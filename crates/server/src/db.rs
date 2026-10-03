//! The server's one SQLite file.

use hab_proto::wire::{DeviceEntry, DeviceRecord, Envelope, Numbered, NumberedEnvelope, SealedKey};
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
                 name           TEXT NOT NULL DEFAULT '',
                 portable       INTEGER NOT NULL,
                 alg            TEXT NOT NULL,
                 signing_public BLOB NOT NULL,
                 sealing_public BLOB NOT NULL,
                 -- The identity key's signature over both public keys.
                 signature      BLOB NOT NULL,
                 created_at     INTEGER NOT NULL
             );
             -- A reminder list the server stores for an account. Only its id is
             -- known: its name, reminders and everything else are inside events.
             CREATE TABLE IF NOT EXISTS lists (
                 id         TEXT PRIMARY KEY,
                 account_id INTEGER NOT NULL REFERENCES accounts(id),
                 created_at INTEGER NOT NULL
             );
             -- The list's key, sealed to a device by another device. The server
             -- cannot open these; it only hands them to the device they are for.
             CREATE TABLE IF NOT EXISTS list_keys (
                 list_id     TEXT    NOT NULL REFERENCES lists(id),
                 device_id   INTEGER NOT NULL REFERENCES devices(id),
                 key_version INTEGER NOT NULL,
                 sealed_by   INTEGER NOT NULL REFERENCES devices(id),
                 alg         TEXT    NOT NULL,
                 encapped    BLOB    NOT NULL,
                 sealed      BLOB    NOT NULL,
                 PRIMARY KEY (list_id, device_id, key_version)
             );
             -- Each list's stream. The server numbers events and sees the list,
             -- the device, the size and when it received them. The ciphertext
             -- is kept so other devices can download it, never read.
             CREATE TABLE IF NOT EXISTS events (
                 list_id     TEXT    NOT NULL REFERENCES lists(id),
                 seq         INTEGER NOT NULL,
                 event_id    TEXT    NOT NULL UNIQUE,
                 device_id   INTEGER NOT NULL REFERENCES devices(id),
                 alg         TEXT    NOT NULL,
                 format      INTEGER NOT NULL,
                 clock       INTEGER NOT NULL,
                 size        INTEGER NOT NULL,
                 received_at INTEGER NOT NULL,
                 nonce       BLOB    NOT NULL,
                 ciphertext  BLOB    NOT NULL,
                 signature   BLOB    NOT NULL,
                 PRIMARY KEY (list_id, seq)
             );
             PRAGMA user_version = 2;",
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
        let device_id = insert_device(&tx, account_id, &new.device, now)?;
        tx.commit()?;
        Ok(Some(Created {
            account_id,
            device_id,
        }))
    }

    /// What a sign-in needs to know about an account.
    pub fn login_account(&self, username: &str) -> Result<Option<LoginAccount>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, display_name, admin, identity_public, opaque_record, kdf_alg,
                        kdf_memory_kib, kdf_passes, kdf_lanes, bundle_alg, bundle_nonce, bundle
                 FROM accounts WHERE username = ?1",
                [username],
                |r| {
                    Ok(LoginAccount {
                        id: r.get(0)?,
                        display_name: r.get(1)?,
                        admin: r.get(2)?,
                        identity_public: r.get(3)?,
                        opaque_record: r.get(4)?,
                        kdf_alg: r.get(5)?,
                        kdf_memory_kib: r.get(6)?,
                        kdf_passes: r.get(7)?,
                        kdf_lanes: r.get(8)?,
                        bundle_alg: r.get(9)?,
                        bundle_nonce: r.get(10)?,
                        bundle: r.get(11)?,
                    })
                },
            )
            .optional()?)
    }

    /// Add a device to an existing account. Returns `None` if the account
    /// already has [`MAX_DEVICES`].
    pub fn add_device(
        &self,
        account_id: i64,
        d: &NewDevice,
        now: i64,
    ) -> Result<Option<i64>, DbError> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM devices WHERE account_id = ?1",
            [account_id],
            |r| r.get(0),
        )?;
        if count >= MAX_DEVICES {
            return Ok(None);
        }
        let id = insert_device(&tx, account_id, d, now)?;
        tx.commit()?;
        Ok(Some(id))
    }

    /// A device, with the keys its requests and events are checked against.
    pub fn device(&self, id: i64) -> Result<Option<DeviceRow>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, account_id, signing_public FROM devices WHERE id = ?1",
                [id],
                |r| {
                    Ok(DeviceRow {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        signing_public: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// Every device of an account, as the join stored it.
    pub fn devices_of(&self, account_id: i64) -> Result<Vec<DeviceEntry>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, portable, alg, signing_public, sealing_public, signature
             FROM devices WHERE account_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            Ok(DeviceEntry {
                id: r.get(0)?,
                record: DeviceRecord {
                    portable: r.get(1)?,
                    alg: r.get(2)?,
                    signing_public: r.get(3)?,
                    sealing_public: r.get(4)?,
                    signature: r.get(5)?,
                },
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Make `list_id` a list of this account and store sealed copies of its
    /// key. Registering again adds copies, such as for a new device. Fails
    /// with `Forbidden` if another account has the list, or if a copy names a
    /// device that isn't this account's.
    pub fn register_list(
        &self,
        account_id: i64,
        list_id: &str,
        keys: &[SealedKey],
        now: i64,
    ) -> Result<(), SyncError> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let owner: Option<i64> = tx
            .query_row(
                "SELECT account_id FROM lists WHERE id = ?1",
                [list_id],
                |r| r.get(0),
            )
            .optional()?;
        match owner {
            Some(o) if o != account_id => return Err(SyncError::Forbidden),
            Some(_) => {}
            None => {
                tx.execute(
                    "INSERT INTO lists (id, account_id, created_at) VALUES (?1, ?2, ?3)",
                    params![list_id, account_id, now],
                )?;
            }
        }
        for k in keys {
            for device in [k.device_id, k.sealed_by] {
                let account: Option<i64> = tx
                    .query_row(
                        "SELECT account_id FROM devices WHERE id = ?1",
                        [device],
                        |r| r.get(0),
                    )
                    .optional()?;
                if account != Some(account_id) {
                    return Err(SyncError::Forbidden);
                }
            }
            tx.execute(
                "INSERT OR REPLACE INTO list_keys
                     (list_id, device_id, key_version, sealed_by, alg, encapped, sealed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    list_id,
                    k.device_id,
                    k.key_version,
                    k.sealed_by,
                    k.alg,
                    k.encapped,
                    k.ciphertext
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The ids of an account's lists, oldest first.
    pub fn lists_of(&self, account_id: i64) -> Result<Vec<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM lists WHERE account_id = ?1 ORDER BY created_at, rowid")?;
        let rows = stmt.query_map([account_id], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The copies of a list's key sealed to `device_id`.
    pub fn sealed_keys(
        &self,
        account_id: i64,
        list_id: &str,
        device_id: i64,
    ) -> Result<Vec<SealedKey>, SyncError> {
        self.owned_list(account_id, list_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT key_version, sealed_by, alg, encapped, sealed FROM list_keys
             WHERE list_id = ?1 AND device_id = ?2 ORDER BY key_version",
        )?;
        let rows = stmt.query_map(params![list_id, device_id], |r| {
            Ok(SealedKey {
                alg: r.get(2)?,
                device_id,
                sealed_by: r.get(1)?,
                key_version: r.get(0)?,
                encapped: r.get(3)?,
                ciphertext: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn owned_list(&self, account_id: i64, list_id: &str) -> Result<(), SyncError> {
        let owner: Option<i64> = self
            .conn
            .query_row(
                "SELECT account_id FROM lists WHERE id = ?1",
                [list_id],
                |r| r.get(0),
            )
            .optional()?;
        match owner {
            Some(o) if o == account_id => Ok(()),
            // The same answer for a list that isn't there and one that isn't
            // theirs, so ids can't be probed.
            _ => Err(SyncError::UnknownList),
        }
    }

    /// Add an event to its list's stream with the next number. Sending the
    /// same event again returns its number; reusing an event id for anything
    /// else is refused.
    pub fn append_event(
        &self,
        account_id: i64,
        e: &Envelope,
        now: i64,
    ) -> Result<Numbered, SyncError> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let owner: Option<i64> = tx
            .query_row(
                "SELECT account_id FROM lists WHERE id = ?1",
                [&e.list_id],
                |r| r.get(0),
            )
            .optional()?;
        if owner != Some(account_id) {
            return Err(SyncError::UnknownList);
        }
        let existing: Option<(String, i64, i64, Vec<u8>, i64)> = tx
            .query_row(
                "SELECT list_id, device_id, seq, signature, received_at FROM events WHERE event_id = ?1",
                [&e.event_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        if let Some((list, device, seq, signature, received_at)) = existing {
            if list == e.list_id && device == e.device_id && signature == e.signature {
                return Ok(Numbered {
                    event_id: e.event_id.clone(),
                    seq,
                    received_at,
                    duplicate: true,
                });
            }
            return Err(SyncError::DuplicateId);
        }
        let seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE list_id = ?1",
            [&e.list_id],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO events (list_id, seq, event_id, device_id, alg, format, clock, size,
                 received_at, nonce, ciphertext, signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                e.list_id,
                seq,
                e.event_id,
                e.device_id,
                e.alg,
                e.format,
                e.clock,
                e.ciphertext.len() as i64,
                now,
                e.nonce,
                e.ciphertext,
                e.signature
            ],
        )?;
        tx.commit()?;
        Ok(Numbered {
            event_id: e.event_id.clone(),
            seq,
            received_at: now,
            duplicate: false,
        })
    }

    /// Up to `limit` events of a list numbered after `after`, and whether more follow.
    pub fn events_after(
        &self,
        account_id: i64,
        list_id: &str,
        after: i64,
        limit: u32,
    ) -> Result<(Vec<NumberedEnvelope>, bool), SyncError> {
        self.owned_list(account_id, list_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT seq, received_at, alg, format, event_id, device_id, clock, nonce, ciphertext, signature
             FROM events WHERE list_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
        )?;
        let mut rows: Vec<NumberedEnvelope> = stmt
            .query_map(params![list_id, after, limit as i64 + 1], |r| {
                Ok(NumberedEnvelope {
                    seq: r.get(0)?,
                    received_at: r.get(1)?,
                    envelope: Envelope {
                        alg: r.get(2)?,
                        format: r.get(3)?,
                        list_id: list_id.to_string(),
                        event_id: r.get(4)?,
                        device_id: r.get(5)?,
                        clock: r.get(6)?,
                        nonce: r.get(7)?,
                        ciphertext: r.get(8)?,
                        signature: r.get(9)?,
                    },
                })
            })?
            .collect::<Result<_, _>>()?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        Ok((rows, more))
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

/// Most devices one account may have.
pub const MAX_DEVICES: i64 = 32;

/// The devices table keeps a `name` column from before names moved into the
/// user's encrypted settings. It is always empty: the server can't read names.
fn insert_device(
    tx: &Transaction,
    account_id: i64,
    d: &NewDevice,
    now: i64,
) -> Result<i64, rusqlite::Error> {
    tx.execute(
        "INSERT INTO devices (account_id, name, portable, alg, signing_public,
             sealing_public, signature, created_at)
         VALUES (?1, '', ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            account_id,
            d.portable,
            d.alg,
            d.signing_public,
            d.sealing_public,
            d.signature,
            now
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

/// An account as a sign-in sees it.
pub struct LoginAccount {
    pub id: i64,
    pub display_name: String,
    pub admin: bool,
    pub identity_public: Vec<u8>,
    pub opaque_record: Vec<u8>,
    pub kdf_alg: String,
    pub kdf_memory_kib: u32,
    pub kdf_passes: u32,
    pub kdf_lanes: u32,
    pub bundle_alg: String,
    pub bundle_nonce: Vec<u8>,
    pub bundle: Vec<u8>,
}

/// A device and the account it belongs to.
pub struct DeviceRow {
    pub id: i64,
    pub account_id: i64,
    pub signing_public: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error("there is no such list")]
    UnknownList,
    #[error("not allowed")]
    Forbidden,
    #[error("that event id is already used")]
    DuplicateId,
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

    fn envelope(list: &str, id: &str, device: i64) -> Envelope {
        Envelope {
            alg: "x".into(),
            format: 9,
            list_id: list.into(),
            event_id: id.into(),
            device_id: device,
            clock: 1,
            nonce: vec![1; 24],
            ciphertext: vec![2; 40],
            signature: vec![3; 64],
        }
    }

    /// Two accounts: the first from joining, the second inserted directly.
    fn two_accounts() -> Db {
        let db = Db::in_memory().unwrap();
        db.create_first_account(|_| true, &test_account("chris"), 0)
            .unwrap()
            .unwrap();
        db.conn
            .execute_batch(
                "INSERT INTO accounts VALUES (2, 'dana', 'Dana', 0, 0, 'i', x'01', x'02', 'argon2id', 1, 1, 1, 'b', x'03', x'04');
                 INSERT INTO devices VALUES (2, 2, 'Phone', 1, 'd', x'05', x'06', x'07', 0);",
            )
            .unwrap();
        db
    }

    #[test]
    fn the_server_numbers_each_list_in_the_order_events_arrive() {
        let db = two_accounts();
        db.register_list(1, "a", &[], 0).unwrap();
        db.register_list(1, "b", &[], 0).unwrap();
        let n = |l: &str, id: &str| db.append_event(1, &envelope(l, id, 1), 10).unwrap().seq;
        assert_eq!(
            (n("a", "e1"), n("a", "e2"), n("b", "e3"), n("a", "e4")),
            (1, 2, 1, 3)
        );
        let (page, more) = db.events_after(1, "a", 1, 10).unwrap();
        assert_eq!(page.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2, 3]);
        assert!(!more);
        let (page, more) = db.events_after(1, "a", 0, 2).unwrap();
        assert_eq!((page.len(), more), (2, true));
        // Any format version is stored as it came.
        assert_eq!(page[0].envelope.format, 9);
    }

    #[test]
    fn a_list_is_only_its_accounts_and_looks_missing_to_others() {
        let db = two_accounts();
        db.register_list(1, "a", &[], 0).unwrap();
        assert!(matches!(
            db.register_list(2, "a", &[], 0),
            Err(SyncError::Forbidden)
        ));
        assert!(matches!(
            db.append_event(2, &envelope("a", "x", 2), 1),
            Err(SyncError::UnknownList)
        ));
        assert!(matches!(
            db.events_after(2, "a", 0, 10),
            Err(SyncError::UnknownList)
        ));
        assert!(matches!(
            db.events_after(2, "nope", 0, 10),
            Err(SyncError::UnknownList)
        ));
        // A sealed copy can't name another account's device.
        let key = SealedKey {
            alg: "k".into(),
            device_id: 2,
            sealed_by: 1,
            key_version: 1,
            encapped: vec![1],
            ciphertext: vec![2],
        };
        assert!(matches!(
            db.register_list(1, "a", &[key], 0),
            Err(SyncError::Forbidden)
        ));
        assert_eq!(db.lists_of(1).unwrap(), vec!["a".to_string()]);
        assert!(db.lists_of(2).unwrap().is_empty());
    }

    #[test]
    fn an_event_id_can_be_resent_but_not_reused() {
        let db = two_accounts();
        db.register_list(1, "a", &[], 0).unwrap();
        db.register_list(1, "b", &[], 0).unwrap();
        let e = envelope("a", "e1", 1);
        assert!(!db.append_event(1, &e, 10).unwrap().duplicate);
        let again = db.append_event(1, &e, 99).unwrap();
        assert_eq!(
            (again.seq, again.received_at, again.duplicate),
            (1, 10, true)
        );
        let mut other = e.clone();
        other.signature = vec![9; 64];
        assert!(matches!(
            db.append_event(1, &other, 11),
            Err(SyncError::DuplicateId)
        ));
        assert!(matches!(
            db.append_event(1, &envelope("b", "e1", 1), 11),
            Err(SyncError::DuplicateId)
        ));
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
