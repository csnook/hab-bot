//! The server's one SQLite file.

use rusqlite::{params, Connection, OptionalExtension};
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
             );
             -- Accounts are filled in by joining (#56); the setup code only
             -- needs to know whether any exists.
             CREATE TABLE IF NOT EXISTS accounts (
                 id         INTEGER PRIMARY KEY,
                 username   TEXT NOT NULL UNIQUE,
                 created_at INTEGER NOT NULL
             );",
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

    /// Insert an account row. Joining (#56) will replace this with the real thing.
    pub fn add_account(&self, username: &str, now: i64) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO accounts (username, created_at) VALUES (?1, ?2)",
            params![username, now],
        )?;
        Ok(())
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
        db.add_account("chris", 0).unwrap();
        assert!(db.account_exists().unwrap());
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
