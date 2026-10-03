use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::event::{Event, StoredEvent, FORMAT_VERSION};
use crate::Result;

/// SQLite storage: the event streams and a few device settings.
///
/// An event's `seq` is its place in its list's stream. A standalone device
/// numbers its own; once joined, new events have no `seq` ("not sent yet")
/// until the server numbers them. `id` is the order this device made them in.
pub struct Store {
    conn: Connection,
}

const SCHEMA_VERSION: i64 = 1;

/// A stored event whose body is kept raw, because it is in a newer format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldEvent {
    pub list_id: String,
    pub event_id: String,
    pub format: u32,
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
             );",
        )?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < SCHEMA_VERSION {
            // The first release's table had no local order and no room for
            // events the server hasn't numbered yet. Keep its rows.
            let old: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'events')",
                [],
                |r| r.get(0),
            )?;
            conn.execute_batch("BEGIN;")?;
            if old {
                conn.execute_batch("ALTER TABLE events RENAME TO events_old;")?;
            }
            conn.execute_batch(
                "CREATE TABLE events (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    list_id     TEXT    NOT NULL,
                    seq         INTEGER,
                    event_id    TEXT    NOT NULL UNIQUE,
                    device_id   TEXT    NOT NULL,
                    author      TEXT    NOT NULL,
                    recorded_at INTEGER NOT NULL,
                    format      INTEGER NOT NULL,
                    body        BLOB    NOT NULL,
                    UNIQUE (list_id, seq)
                 );",
            )?;
            if old {
                conn.execute_batch(
                    "INSERT INTO events (list_id, seq, event_id, device_id, author, recorded_at, format, body)
                     SELECT list_id, seq, event_id, device_id, author, recorded_at, format, CAST(body AS BLOB)
                     FROM events_old ORDER BY list_id, seq;
                     DROP TABLE events_old;",
                )?;
            }
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"))?;
        }
        Ok(Store { conn })
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Every stored setting whose key starts with `prefix`, with the prefix
    /// left on.
    pub fn meta_prefix(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM meta WHERE substr(key, 1, length(?1)) = ?1")?;
        let rows = stmt.query_map([prefix], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Removes every stored setting whose key starts with `prefix`.
    pub fn delete_meta_prefix(&self, prefix: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM meta WHERE substr(key, 1, length(?1)) = ?1",
            [prefix],
        )?;
        Ok(())
    }

    /// A stored setting, created with `make` on first use.
    pub fn meta_or_init(&self, key: &str, make: impl FnOnce() -> String) -> Result<String> {
        if let Some(v) = self.meta(key)? {
            return Ok(v);
        }
        let v = make();
        self.set_meta(key, &v)?;
        Ok(v)
    }

    /// Whether this device has joined a server, which then numbers its events.
    pub fn joined(&self) -> Result<bool> {
        Ok(self.meta("joined")?.as_deref() == Some("1"))
    }

    /// Appends an event to a list's stream. A standalone device numbers it
    /// next. A joined device leaves it unnumbered, "not sent yet".
    pub fn append(
        &mut self,
        list_id: &str,
        device_id: &str,
        author: &str,
        recorded_at: i64,
        event: Event,
        event_id: String,
    ) -> Result<StoredEvent> {
        let joined = self.joined()?;
        let tx = self.conn.transaction()?;
        let seq: Option<i64> = if joined {
            None
        } else {
            Some(tx.query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE list_id = ?1",
                [list_id],
                |r| r.get(0),
            )?)
        };
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
                event.format(),
                serde_json::to_vec(&event)?
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

    /// Stores an event another device made, numbered by the server. Returns
    /// false if this device already had it (such as its own event coming
    /// back), in which case it only gains the number.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_numbered(
        &mut self,
        list_id: &str,
        seq: i64,
        event_id: &str,
        device_id: &str,
        author: &str,
        recorded_at: i64,
        format: u32,
        body: &[u8],
    ) -> Result<bool> {
        let tx = self.conn.transaction()?;
        let have: Option<Option<i64>> = tx
            .query_row(
                "SELECT seq FROM events WHERE event_id = ?1",
                [event_id],
                |r| r.get(0),
            )
            .optional()?;
        let inserted = match have {
            Some(existing) => {
                if existing.is_none() {
                    tx.execute(
                        "UPDATE events SET seq = ?2 WHERE event_id = ?1",
                        params![event_id, seq],
                    )?;
                }
                false
            }
            None => {
                tx.execute(
                    "INSERT INTO events (list_id, seq, event_id, device_id, author, recorded_at, format, body)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![list_id, seq, event_id, device_id, author, recorded_at, format, body],
                )?;
                true
            }
        };
        tx.commit()?;
        Ok(inserted)
    }

    /// Drops the numbers a standalone device gave its events, so the server's
    /// can replace them when it numbers what is uploaded.
    pub fn forget_local_numbers(&self, list_id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE events SET seq = NULL WHERE list_id = ?1", [list_id])?;
        self.conn.execute(
            "DELETE FROM meta WHERE key = ?1",
            [format!("cursor:{list_id}")],
        )?;
        Ok(())
    }

    /// The server numbered one of this device's events.
    pub fn set_seq(&self, event_id: &str, seq: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE events SET seq = ?2 WHERE event_id = ?1 AND seq IS NULL",
            params![event_id, seq],
        )?;
        Ok(())
    }

    /// Takes on a server account's identity: the user and device that now
    /// make the events, including the ones made while standalone. From now on
    /// the server numbers new events.
    pub fn adopt(&mut self, user_id: &str, device_id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE events SET author = ?1, device_id = ?2",
            params![user_id, device_id],
        )?;
        for (k, v) in [
            ("user_id", user_id),
            ("device_id", device_id),
            ("joined", "1"),
        ] {
            tx.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The events of a list that this app can apply, in order: numbered ones
    /// by number, then unsent ones in the order they were made.
    pub fn stream(&self, list_id: &str) -> Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, event_id, device_id, author, recorded_at, body
             FROM events WHERE list_id = ?1 AND format <= ?2
             ORDER BY seq IS NULL, seq, id",
        )?;
        let rows = stmt.query_map(params![list_id, FORMAT_VERSION], |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Vec<u8>>(5)?,
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
                event: serde_json::from_slice(&body)?,
            });
        }
        Ok(out)
    }

    /// Events kept without being applied, because a newer app made them.
    pub fn held(&self, list_id: &str) -> Result<Vec<HeldEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, format FROM events WHERE list_id = ?1 AND format > ?2 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![list_id, FORMAT_VERSION], |r| {
            Ok(HeldEvent {
                list_id: list_id.to_string(),
                event_id: r.get(0)?,
                format: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Events not yet numbered by the server, as `(event_id, format,
    /// recorded_at, author, body)`, in the order they were made.
    pub fn unsent(&self, list_id: &str) -> Result<Vec<UnsentRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, format, recorded_at, author, body
             FROM events WHERE list_id = ?1 AND seq IS NULL ORDER BY id",
        )?;
        let rows = stmt.query_map([list_id], |r| {
            Ok(UnsentRow {
                event_id: r.get(0)?,
                format: r.get(1)?,
                recorded_at: r.get(2)?,
                author: r.get(3)?,
                body: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Every list that has events.
    pub fn list_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT list_id FROM events ORDER BY list_id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

pub struct UnsentRow {
    pub event_id: String,
    pub format: u32,
    pub recorded_at: i64,
    pub author: String,
    pub body: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn created(n: i64) -> Event {
        Event::ReminderCreated {
            reminder_id: format!("r{n}"),
            title: "T".into(),
            fire_at: n,
        }
    }

    #[test]
    fn standalone_events_are_numbered_locally_and_joined_ones_wait_for_the_server() {
        let mut s = Store::open_in_memory().unwrap();
        let a = s.append("l", "d", "u", 1, created(1), "e1".into()).unwrap();
        assert_eq!(a.seq, Some(1));
        s.adopt("user", "7").unwrap();
        let b = s
            .append("l", "7", "user", 2, created(2), "e2".into())
            .unwrap();
        assert_eq!(b.seq, None);
        // Adopting rewrote who made the earlier event.
        let stream = s.stream("l").unwrap();
        assert_eq!(stream[0].author, "user");
        assert_eq!(stream[0].device_id, "7");
        assert_eq!(s.unsent("l").unwrap().len(), 1);
        s.set_seq("e2", 5).unwrap();
        assert!(s.unsent("l").unwrap().is_empty());
        assert_eq!(s.stream("l").unwrap()[1].seq, Some(5));
    }

    #[test]
    fn numbered_events_come_before_unsent_ones() {
        let mut s = Store::open_in_memory().unwrap();
        s.adopt("user", "7").unwrap();
        s.append("l", "7", "user", 1, created(1), "mine".into())
            .unwrap();
        let body = serde_json::to_vec(&created(2)).unwrap();
        assert!(s
            .insert_numbered("l", 1, "theirs", "8", "user", 2, 1, &body)
            .unwrap());
        let ids: Vec<_> = s
            .stream("l")
            .unwrap()
            .into_iter()
            .map(|e| e.event_id)
            .collect();
        assert_eq!(ids, ["theirs", "mine"]);
        // Its own event coming back only gains a number.
        assert!(!s
            .insert_numbered("l", 2, "mine", "7", "user", 1, 1, &body)
            .unwrap());
        assert_eq!(s.stream("l").unwrap()[1].seq, Some(2));
    }

    #[test]
    fn a_newer_format_is_kept_but_not_in_the_stream() {
        let mut s = Store::open_in_memory().unwrap();
        s.insert_numbered(
            "l",
            1,
            "future",
            "8",
            "u",
            1,
            FORMAT_VERSION + 1,
            b"{\"new\":1}",
        )
        .unwrap();
        assert!(s.stream("l").unwrap().is_empty());
        assert_eq!(
            s.held("l").unwrap(),
            [HeldEvent {
                list_id: "l".into(),
                event_id: "future".into(),
                format: FORMAT_VERSION + 1
            }]
        );
    }

    #[test]
    fn the_first_release_database_is_upgraded_with_its_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE events (
                    list_id TEXT NOT NULL, seq INTEGER NOT NULL, event_id TEXT NOT NULL UNIQUE,
                    device_id TEXT NOT NULL, author TEXT NOT NULL, recorded_at INTEGER NOT NULL,
                    format INTEGER NOT NULL, body TEXT NOT NULL, PRIMARY KEY (list_id, seq));
                 INSERT INTO events VALUES ('l', 1, 'e1', 'd', 'u', 5, 1,
                    '{\"type\":\"reminder_created\",\"reminder_id\":\"r\",\"title\":\"Old\",\"fire_at\":9}');",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let stream = s.stream("l").unwrap();
        assert_eq!(stream.len(), 1);
        assert_eq!(stream[0].seq, Some(1));
        assert_eq!(
            stream[0].event,
            Event::ReminderCreated {
                reminder_id: "r".into(),
                title: "Old".into(),
                fire_at: 9
            }
        );
        drop(s);
        // Opening again does not migrate twice.
        assert_eq!(Store::open(&path).unwrap().stream("l").unwrap().len(), 1);
    }
}
