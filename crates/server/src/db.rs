//! The server's one SQLite file.

use hab_proto::wire::{
    DeviceEntry, DeviceRecord, Envelope, Numbered, NumberedEnvelope, Rotation, SealedKey,
};
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
        // Events used to name their device as a foreign key, which stops a
        // device being removed while its events remain. Rebuild such a table.
        let events_name_devices: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('events') WHERE \"table\" = 'devices'",
            [],
            |r| r.get(0),
        )?;
        if events_name_devices > 0 {
            conn.execute_batch(
                "ALTER TABLE events RENAME TO events_old;
                 CREATE TABLE events (
                     list_id     TEXT    NOT NULL REFERENCES lists(id),
                     seq         INTEGER NOT NULL,
                     event_id    TEXT    NOT NULL UNIQUE,
                     device_id   INTEGER NOT NULL,
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
                 INSERT INTO events SELECT * FROM events_old;
                 DROP TABLE events_old;",
            )?;
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
                 -- Not a foreign key: a removed device's events stay, and its
                 -- row in `devices` does not.
                 device_id   INTEGER NOT NULL,
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
             -- When the server last heard from each device. Timing only.
             CREATE TABLE IF NOT EXISTS device_activity (
                 device_id      INTEGER PRIMARY KEY,
                 last_synced_at INTEGER NOT NULL
             );
             -- Devices that were removed from their account. The row in
             -- `devices` is gone, so the device can no longer sign a request
             -- or be sealed a key. What it signed before still has to verify.
             CREATE TABLE IF NOT EXISTS retired_devices (
                 id             INTEGER PRIMARY KEY,
                 account_id     INTEGER NOT NULL,
                 portable       INTEGER NOT NULL,
                 alg            TEXT NOT NULL,
                 signing_public BLOB NOT NULL,
                 sealing_public BLOB NOT NULL,
                 signature      BLOB NOT NULL,
                 removed_at     INTEGER NOT NULL
             );
             -- What the server tells an account's devices itself, such as repeated
             -- failed sign-ins. A kind, a number and a time: never an address.
             CREATE TABLE IF NOT EXISTS notices (
                 id         INTEGER PRIMARY KEY,
                 account_id INTEGER NOT NULL REFERENCES accounts(id),
                 kind       TEXT    NOT NULL,
                 count      INTEGER NOT NULL,
                 created_at INTEGER NOT NULL
             );
             PRAGMA user_version = 4;",
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

    pub fn username_of(&self, account_id: i64) -> Result<Option<String>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT username FROM accounts WHERE id = ?1",
                [account_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// An account's identity key, and whether it is a server admin.
    pub fn identity_of(&self, account_id: i64) -> Result<Option<(Vec<u8>, bool)>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT identity_public, admin FROM accounts WHERE id = ?1",
                [account_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Replace an account's password: the OPAQUE record, the hardening cost
    /// and the key bundle sealed under the new password. The identity key, the
    /// devices and everything else stay as they are.
    pub fn set_password(
        &self,
        account_id: i64,
        opaque_record: &[u8],
        kdf: &hab_proto::wire::Kdf,
        bundle: &hab_proto::wire::KeyBundle,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE accounts SET opaque_record = ?2, kdf_alg = ?3, kdf_memory_kib = ?4,
                 kdf_passes = ?5, kdf_lanes = ?6, bundle_alg = ?7, bundle_nonce = ?8, bundle = ?9
             WHERE id = ?1",
            params![
                account_id,
                opaque_record,
                kdf.alg,
                kdf.memory_kib,
                kdf.passes,
                kdf.lanes,
                bundle.alg,
                bundle.nonce,
                bundle.ciphertext,
            ],
        )?;
        Ok(())
    }

    /// Keep a notice for the account's devices and return it. Old ones go:
    /// at most [`MAX_NOTICES`] an account, none older than [`NOTICE_KEEP_SECS`].
    pub fn add_notice(
        &self,
        account_id: i64,
        kind: &str,
        count: u32,
        now: i64,
    ) -> Result<hab_proto::wire::ServerNotice, DbError> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO notices (account_id, kind, count, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![account_id, kind, count, now],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "DELETE FROM notices WHERE account_id = ?1 AND (created_at < ?2 OR id NOT IN
                 (SELECT id FROM notices WHERE account_id = ?1 ORDER BY id DESC LIMIT ?3))",
            params![account_id, now - NOTICE_KEEP_SECS, MAX_NOTICES],
        )?;
        tx.commit()?;
        Ok(hab_proto::wire::ServerNotice {
            id,
            kind: kind.into(),
            count,
            at: now,
        })
    }

    /// The account's notices numbered after `after`, oldest first.
    pub fn notices_after(
        &self,
        account_id: i64,
        after: i64,
    ) -> Result<Vec<hab_proto::wire::ServerNotice>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, count, created_at FROM notices
             WHERE account_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, after, MAX_NOTICES], |r| {
            Ok(hab_proto::wire::ServerNotice {
                id: r.get(0)?,
                kind: r.get(1)?,
                count: r.get(2)?,
                at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The newest notice's number, or 0.
    pub fn latest_notice(&self, account_id: i64) -> Result<i64, DbError> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM notices WHERE account_id = ?1",
            [account_id],
            |r| r.get(0),
        )?)
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

    /// Every device of an account, as the join stored it, with when the
    /// server last heard from it.
    pub fn devices_of(&self, account_id: i64) -> Result<Vec<DeviceEntry>, DbError> {
        self.entries(
            "SELECT d.id, d.portable, d.alg, d.signing_public, d.sealing_public, d.signature,
                    a.last_synced_at
             FROM devices d LEFT JOIN device_activity a ON a.device_id = d.id
             WHERE d.account_id = ?1 ORDER BY d.id",
            account_id,
        )
    }

    /// The account's removed devices, kept to verify what they signed.
    pub fn retired_of(&self, account_id: i64) -> Result<Vec<DeviceEntry>, DbError> {
        self.entries(
            "SELECT r.id, r.portable, r.alg, r.signing_public, r.sealing_public, r.signature,
                    a.last_synced_at
             FROM retired_devices r LEFT JOIN device_activity a ON a.device_id = r.id
             WHERE r.account_id = ?1 ORDER BY r.id",
            account_id,
        )
    }

    fn entries(&self, sql: &str, account_id: i64) -> Result<Vec<DeviceEntry>, DbError> {
        let mut stmt = self.conn.prepare(sql)?;
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
                last_synced: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Note that the device was heard from at `now`. Writes at most once in
    /// [`ACTIVITY_GRANULARITY_SECS`] per device.
    pub fn touch_device(&self, device_id: i64, now: i64) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO device_activity (device_id, last_synced_at) VALUES (?1, ?2)
             ON CONFLICT(device_id) DO UPDATE SET last_synced_at = excluded.last_synced_at
             WHERE excluded.last_synced_at >= last_synced_at + ?3",
            params![device_id, now, ACTIVITY_GRANULARITY_SECS],
        )?;
        Ok(())
    }

    /// Take `target` off the account and store the rotated keys, together or
    /// not at all.
    ///
    /// `remover` is the device asking and seals every copy. Each list of the
    /// account must be rotated: the new version is one more than its newest,
    /// and every device that remains must end up with a copy of every version
    /// (a copy the removed device sealed no longer counts, since nobody will
    /// trust it). The removed device's own copies are deleted, its row is
    /// replaced by a retired record, and its events stay.
    pub fn remove_device(
        &self,
        account_id: i64,
        remover: i64,
        target: i64,
        rotations: &[Rotation],
        now: i64,
    ) -> Result<(), RemoveError> {
        if target == remover {
            return Err(RemoveError::OwnDevice);
        }
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let on_account: Option<i64> = tx
            .query_row(
                "SELECT account_id FROM devices WHERE id = ?1",
                [target],
                |r| r.get(0),
            )
            .optional()?;
        if on_account != Some(account_id) {
            return Err(RemoveError::UnknownDevice);
        }
        let remaining: Vec<i64> = {
            let mut stmt = tx
                .prepare("SELECT id FROM devices WHERE account_id = ?1 AND id <> ?2 ORDER BY id")?;
            let rows = stmt.query_map(params![account_id, target], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut lists: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM lists WHERE account_id = ?1 ORDER BY id")?;
            let rows = stmt.query_map([account_id], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut named: Vec<&str> = rotations.iter().map(|r| r.list_id.as_str()).collect();
        named.sort_unstable();
        lists.sort_unstable();
        if named != lists {
            return Err(RemoveError::Incomplete);
        }
        let mut new_versions = Vec::new();
        for rotation in rotations {
            let newest: Option<u32> = tx.query_row(
                "SELECT MAX(key_version) FROM list_keys WHERE list_id = ?1",
                [&rotation.list_id],
                |r| r.get(0),
            )?;
            let new_version = newest.unwrap_or(0) + 1;
            new_versions.push((rotation.list_id.as_str(), new_version));
            let mut fresh = std::collections::BTreeSet::new();
            for k in &rotation.keys {
                if k.sealed_by != remover
                    || !remaining.contains(&k.device_id)
                    || k.key_version == 0
                    || k.key_version > new_version
                {
                    return Err(RemoveError::Incomplete);
                }
                if k.key_version == new_version {
                    fresh.insert(k.device_id);
                }
                tx.execute(
                    "INSERT OR REPLACE INTO list_keys
                         (list_id, device_id, key_version, sealed_by, alg, encapped, sealed)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        rotation.list_id,
                        k.device_id,
                        k.key_version,
                        k.sealed_by,
                        k.alg,
                        k.encapped,
                        k.ciphertext
                    ],
                )?;
            }
            // Someone else may have rotated, or a device may have signed in,
            // since the caller looked.
            if fresh.len() != remaining.len() {
                return Err(RemoveError::Conflict);
            }
        }
        tx.execute(
            "DELETE FROM list_keys WHERE device_id = ?1 OR sealed_by = ?1",
            [target],
        )?;
        // Every remaining device must hold every version of every list, the
        // old ones included: a copy the removed device sealed was just deleted.
        for (list, versions) in new_versions {
            for device in &remaining {
                let held: i64 = tx.query_row(
                    "SELECT COUNT(DISTINCT key_version) FROM list_keys
                     WHERE list_id = ?1 AND device_id = ?2",
                    params![list, device],
                    |r| r.get(0),
                )?;
                if held != i64::from(versions) {
                    return Err(RemoveError::Incomplete);
                }
            }
        }
        tx.execute(
            "INSERT INTO retired_devices
                 (id, account_id, portable, alg, signing_public, sealing_public, signature, removed_at)
             SELECT id, account_id, portable, alg, signing_public, sealing_public, signature, ?2
             FROM devices WHERE id = ?1",
            params![target, now],
        )?;
        tx.execute("DELETE FROM devices WHERE id = ?1", [target])?;
        tx.commit()?;
        Ok(())
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

/// A device's last-heard-from time is rewritten at most this often.
pub const ACTIVITY_GRANULARITY_SECS: i64 = 30;

/// Why a device could not be removed.
#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error("a device can only be removed from another device")]
    OwnDevice,
    #[error("there is no such device on this account")]
    UnknownDevice,
    #[error("the new keys do not cover every list and every remaining device")]
    Incomplete,
    #[error("the account's devices or keys changed; try again")]
    Conflict,
}

/// Most devices one account may have.
pub const MAX_DEVICES: i64 = 32;
/// Notices kept for an account, and for how long.
pub const MAX_NOTICES: i64 = 50;
pub const NOTICE_KEEP_SECS: i64 = 30 * 24 * 3600;

/// The devices table keeps a `name` column from before names moved into the
/// user's encrypted settings. It is always empty: the server can't read names.
fn insert_device(
    tx: &Transaction,
    account_id: i64,
    d: &NewDevice,
    now: i64,
) -> Result<i64, rusqlite::Error> {
    // Ids are never reused, even after the newest device is removed: events
    // and sealed keys name devices by id.
    let id: i64 = tx.query_row(
        "SELECT COALESCE(MAX(m), 0) + 1 FROM
             (SELECT MAX(id) AS m FROM devices UNION ALL SELECT MAX(id) FROM retired_devices)",
        [],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO devices (id, account_id, name, portable, alg, signing_public,
             sealing_public, signature, created_at)
         VALUES (?8, ?1, '', ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            account_id,
            d.portable,
            d.alg,
            d.signing_public,
            d.sealing_public,
            d.signature,
            now,
            id
        ],
    )?;
    Ok(id)
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
    fn notices_are_numbered_per_account_and_old_ones_go() {
        let db = Db::in_memory().unwrap();
        let a = db
            .create_first_account(|_| true, &test_account("chris"), 0)
            .unwrap()
            .unwrap()
            .account_id;
        assert_eq!(db.latest_notice(a).unwrap(), 0);
        let n1 = db.add_notice(a, "failed_sign_ins", 5, 100).unwrap();
        let n2 = db.add_notice(a, "failed_sign_ins", 10, 200).unwrap();
        assert!(n2.id > n1.id);
        assert_eq!((n2.count, n2.at), (10, 200));
        assert_eq!(db.notices_after(a, 0).unwrap(), [n1.clone(), n2.clone()]);
        assert_eq!(
            db.notices_after(a, n1.id).unwrap(),
            std::slice::from_ref(&n2)
        );
        assert_eq!(db.latest_notice(a).unwrap(), n2.id);
        // A month on, the old ones are dropped.
        let n3 = db
            .add_notice(a, "failed_sign_ins", 5, 200 + NOTICE_KEEP_SECS + 1)
            .unwrap();
        assert_eq!(db.notices_after(a, 0).unwrap(), [n3]);
        // And no more than the most recent are kept.
        for i in 0..MAX_NOTICES + 5 {
            db.add_notice(a, "failed_sign_ins", 5, 300_000_000 + i)
                .unwrap();
        }
        assert_eq!(db.notices_after(a, 0).unwrap().len() as i64, MAX_NOTICES);
    }

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

    /// A key of `list` at `version` for `device`, sealed by `by`.
    fn key(device: i64, by: i64, version: u32) -> SealedKey {
        SealedKey {
            alg: "k".into(),
            device_id: device,
            sealed_by: by,
            key_version: version,
            encapped: vec![1],
            ciphertext: vec![version as u8],
        }
    }

    /// One account with devices 1, 2 and 3 and a list "a" at version 1.
    fn three_devices() -> Db {
        let db = Db::in_memory().unwrap();
        db.create_first_account(|_| true, &test_account("chris"), 0)
            .unwrap()
            .unwrap();
        db.add_device(1, &test_account("x").device, 1).unwrap();
        db.add_device(1, &test_account("y").device, 2).unwrap();
        let v1: Vec<_> = [1, 2, 3].iter().map(|d| key(*d, 3, 1)).collect();
        db.register_list(1, "a", &v1, 0).unwrap();
        db
    }

    fn rotation(from: i64, to: &[i64], versions: &[u32]) -> Vec<Rotation> {
        let keys = to
            .iter()
            .flat_map(|d| versions.iter().map(move |v| key(*d, from, *v)))
            .collect();
        vec![Rotation {
            list_id: "a".into(),
            keys,
        }]
    }

    #[test]
    fn removing_a_device_rotates_every_list_and_keeps_its_events_verifiable() {
        let db = three_devices();
        db.append_event(1, &envelope("a", "e1", 3), 5).unwrap();
        db.touch_device(3, 100).unwrap();
        db.remove_device(1, 1, 3, &rotation(1, &[1, 2], &[1, 2]), 50)
            .unwrap();
        // The row is gone, and the copies it held or sealed.
        assert_eq!(
            db.devices_of(1)
                .unwrap()
                .iter()
                .map(|d| d.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(db.device(3).unwrap().is_none());
        for d in [1, 2] {
            let keys = db.sealed_keys(1, "a", d).unwrap();
            assert_eq!(
                keys.iter().map(|k| k.key_version).collect::<Vec<_>>(),
                vec![1, 2]
            );
            assert!(keys.iter().all(|k| k.sealed_by == 1));
        }
        assert!(db.sealed_keys(1, "a", 3).unwrap().is_empty());
        // Its record is kept to verify what it signed, and its event stays.
        let retired = db.retired_of(1).unwrap();
        assert_eq!(retired.len(), 1);
        assert_eq!((retired[0].id, retired[0].last_synced), (3, Some(100)));
        assert_eq!(db.events_after(1, "a", 0, 10).unwrap().0.len(), 1);
        // A new device never takes the removed one's id.
        let next = db.add_device(1, &test_account("z").device, 60).unwrap();
        assert_eq!(next, Some(4));
    }

    #[test]
    fn a_removal_that_does_not_cover_everything_changes_nothing() {
        let db = three_devices();
        let check_unchanged = |db: &Db| {
            assert_eq!(db.devices_of(1).unwrap().len(), 3);
            assert_eq!(db.sealed_keys(1, "a", 2).unwrap().len(), 1);
            assert!(db.retired_of(1).unwrap().is_empty());
        };
        // Not every list, no copy for a remaining device, a copy sealed by
        // someone else, a copy for the removed device, a version from the
        // future, and a missing older version.
        let cases: Vec<(Vec<Rotation>, &str)> = vec![
            (vec![], "no lists"),
            (rotation(1, &[1], &[1, 2]), "device 2 left out"),
            (rotation(2, &[1, 2], &[1, 2]), "sealed by another device"),
            (
                rotation(1, &[1, 2, 3], &[1, 2]),
                "sealed to the removed device",
            ),
            (rotation(1, &[1, 2], &[3]), "a version that skips one"),
            (rotation(1, &[1, 2], &[1]), "no new version"),
            (rotation(1, &[1, 2], &[2]), "version 1 missing for device 2"),
        ];
        for (rotations, why) in cases {
            let err = db.remove_device(1, 1, 3, &rotations, 9).unwrap_err();
            assert!(
                matches!(err, RemoveError::Incomplete | RemoveError::Conflict),
                "{why}: {err:?}"
            );
            check_unchanged(&db);
        }
        // Itself, or a device that isn't there.
        assert!(matches!(
            db.remove_device(1, 1, 1, &rotation(1, &[2, 3], &[1, 2]), 9),
            Err(RemoveError::OwnDevice)
        ));
        assert!(matches!(
            db.remove_device(1, 1, 77, &rotation(1, &[1, 2], &[1, 2]), 9),
            Err(RemoveError::UnknownDevice)
        ));
        check_unchanged(&db);
    }

    #[test]
    fn a_device_of_another_account_cannot_be_removed() {
        let db = two_accounts();
        db.add_device(1, &test_account("x").device, 1).unwrap();
        db.register_list(1, "a", &[], 0).unwrap();
        // Device 2 is Dana's. Chris asking from device 1 is told it isn't there.
        let rotations = vec![Rotation {
            list_id: "a".into(),
            keys: vec![key(1, 1, 1), key(3, 1, 1)],
        }];
        assert!(matches!(
            db.remove_device(1, 1, 2, &rotations, 9),
            Err(RemoveError::UnknownDevice)
        ));
        assert!(db.device(2).unwrap().is_some());
    }

    #[test]
    fn a_second_rotation_needs_the_first_one_to_be_seen() {
        let db = three_devices();
        db.remove_device(1, 1, 3, &rotation(1, &[1, 2], &[1, 2]), 5)
            .unwrap();
        // Someone who didn't know about version 2 tries to make it again.
        db.add_device(1, &test_account("w").device, 6).unwrap();
        let stale = rotation(1, &[1, 2, 4], &[1, 2]);
        assert!(matches!(
            db.remove_device(1, 1, 2, &stale, 7),
            Err(RemoveError::Conflict) | Err(RemoveError::Incomplete)
        ));
        assert_eq!(db.devices_of(1).unwrap().len(), 3);
        let fresh = rotation(1, &[1, 4], &[1, 2, 3]);
        db.remove_device(1, 1, 2, &fresh, 8).unwrap();
        assert_eq!(db.retired_of(1).unwrap().len(), 2);
    }

    #[test]
    fn last_synced_is_written_at_most_every_half_minute() {
        let db = three_devices();
        let at = |db: &Db| db.devices_of(1).unwrap()[0].last_synced;
        assert_eq!(at(&db), None);
        db.touch_device(1, 100).unwrap();
        db.touch_device(1, 110).unwrap();
        assert_eq!(at(&db), Some(100));
        db.touch_device(1, 131).unwrap();
        assert_eq!(at(&db), Some(131));
    }

    #[test]
    fn events_that_named_their_device_as_a_foreign_key_are_kept_when_migrated() {
        let dir = tempfile::tempdir().unwrap();
        {
            let db = Db::open(dir.path()).unwrap();
            db.create_first_account(|_| true, &test_account("chris"), 0)
                .unwrap()
                .unwrap();
            db.register_list(1, "a", &[], 0).unwrap();
            db.append_event(1, &envelope("a", "e1", 1), 5).unwrap();
            // Put the table back as the previous version made it.
            db.conn
                .execute_batch(
                    "ALTER TABLE events RENAME TO events_new;
                     CREATE TABLE events (
                         list_id TEXT NOT NULL REFERENCES lists(id), seq INTEGER NOT NULL,
                         event_id TEXT NOT NULL UNIQUE,
                         device_id INTEGER NOT NULL REFERENCES devices(id),
                         alg TEXT NOT NULL, format INTEGER NOT NULL, clock INTEGER NOT NULL,
                         size INTEGER NOT NULL, received_at INTEGER NOT NULL, nonce BLOB NOT NULL,
                         ciphertext BLOB NOT NULL, signature BLOB NOT NULL,
                         PRIMARY KEY (list_id, seq));
                     INSERT INTO events SELECT * FROM events_new;
                     DROP TABLE events_new;
                     PRAGMA user_version = 2;",
                )
                .unwrap();
        }
        let db = Db::open(dir.path()).unwrap();
        assert_eq!(db.events_after(1, "a", 0, 10).unwrap().0.len(), 1);
        // And a device with events can now be removed.
        db.add_device(1, &test_account("x").device, 1).unwrap();
        db.register_list(1, "a", &[key(1, 1, 1), key(2, 1, 1)], 0)
            .unwrap();
        db.remove_device(1, 2, 1, &rotation(2, &[2], &[1, 2]), 9)
            .unwrap();
        assert_eq!(db.retired_of(1).unwrap().len(), 1);
        assert_eq!(db.events_after(1, "a", 0, 10).unwrap().0.len(), 1);
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
