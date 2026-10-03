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
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                seq     INTEGER PRIMARY KEY AUTOINCREMENT,
                list_id TEXT NOT NULL,
                body    TEXT NOT NULL
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
