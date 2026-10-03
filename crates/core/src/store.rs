use crate::{Event, Result};
use rusqlite::{params, Connection};

/// The event streams, stored in SQLite.
pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &str) -> Result<Store> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        // The Android alarm path opens its own connection while the app may be open.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                seq     INTEGER PRIMARY KEY AUTOINCREMENT,
                list_id TEXT NOT NULL,
                body    TEXT NOT NULL
            );
            -- state that belongs to this device and isn't synced, such as what its alerts
            -- have shown so far
            CREATE TABLE IF NOT EXISTS device_state (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;
        Ok(Store { conn })
    }

    pub fn append(&self, list_id: &str, event: &Event) -> Result<()> {
        self.conn.execute(
            "INSERT INTO events (list_id, body) VALUES (?1, ?2)",
            params![list_id, serde_json::to_string(event)?],
        )?;
        Ok(())
    }

    pub fn device_state(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM device_state WHERE key = ?1")?;
        let mut rows = stmt.query_map(params![key], |row| row.get::<_, String>(0))?;
        Ok(rows.next().transpose()?)
    }

    pub fn set_device_state(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO device_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// The list's stream, in order.
    pub fn events(&self, list_id: &str) -> Result<Vec<Event>> {
        let mut stmt = self
            .conn
            .prepare("SELECT body FROM events WHERE list_id = ?1 ORDER BY seq")?;
        let bodies = stmt.query_map(params![list_id], |row| row.get::<_, String>(0))?;
        let mut events = Vec::new();
        for body in bodies {
            events.push(serde_json::from_str(&body?)?);
        }
        Ok(events)
    }
}
