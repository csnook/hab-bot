use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::event::{Event, StoredEvent, FORMAT_VERSION};
use crate::Result;

/// SQLite storage: the event streams and a few device settings.
pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS events (
                list_id     TEXT    NOT NULL,
                seq         INTEGER NOT NULL,
                event_id    TEXT    NOT NULL UNIQUE,
                device_id   TEXT    NOT NULL,
                author      TEXT    NOT NULL,
                recorded_at INTEGER NOT NULL,
                format      INTEGER NOT NULL,
                body        TEXT    NOT NULL,
                PRIMARY KEY (list_id, seq)
             );",
        )?;
        Ok(Store { conn })
    }

    /// A stored setting, created with `make` on first use.
    pub fn meta_or_init(&self, key: &str, make: impl FnOnce() -> String) -> Result<String> {
        let existing: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?;
        if let Some(v) = existing {
            return Ok(v);
        }
        let v = make();
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)",
            params![key, v],
        )?;
        Ok(v)
    }

    /// Appends an event to a list's stream, numbering it next.
    pub fn append(
        &mut self,
        list_id: &str,
        device_id: &str,
        author: &str,
        recorded_at: i64,
        event: Event,
        event_id: String,
    ) -> Result<StoredEvent> {
        let tx = self.conn.transaction()?;
        let seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE list_id = ?1",
            [list_id],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO events (list_id, seq, event_id, device_id, author, recorded_at, format, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                list_id,
                seq,
                event_id,
                device_id,
                author,
                recorded_at,
                FORMAT_VERSION,
                serde_json::to_string(&event)?
            ],
        )?;
        tx.commit()?;
        Ok(StoredEvent {
            list_id: list_id.to_string(),
            seq,
            event_id,
            device_id: device_id.to_string(),
            author: author.to_string(),
            recorded_at,
            event,
        })
    }

    /// Every event of a list's stream, in order.
    pub fn stream(&self, list_id: &str) -> Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, event_id, device_id, author, recorded_at, body
             FROM events WHERE list_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([list_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, event_id, device_id, author, recorded_at, body) = row?;
            out.push(StoredEvent {
                list_id: list_id.to_string(),
                seq,
                event_id,
                device_id,
                author,
                recorded_at,
                event: serde_json::from_str(&body)?,
            });
        }
        Ok(out)
    }
}
