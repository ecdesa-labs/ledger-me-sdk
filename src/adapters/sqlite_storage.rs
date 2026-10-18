use rusqlite::{params, Connection};
use std::sync::Mutex;

use crate::ports::Storage;

/// [`Storage`] backed by a single-table SQLite database.
///
/// `rusqlite::Connection` isn't `Sync`, so the connection is guarded by a
/// [`Mutex`] to let `SqliteStorage` be shared across pallet calls (via
/// `Arc<dyn Storage>`) despite `Storage`'s methods taking `&self`; this
/// serializes all reads and writes on the connection.
pub struct SqliteStorage {
    connection: Mutex<Connection>,
}

impl SqliteStorage {
    /// Opens (or creates) the SQLite database at `path` and ensures its
    /// `state` table exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection can't be opened or the table
    /// can't be created.
    pub fn new(path: &str) -> anyhow::Result<Self> {
        let connection = Connection::open(path)?;

        connection.execute(
            "
            CREATE TABLE IF NOT EXISTS state (
                key   BLOB PRIMARY KEY,
                value BLOB NOT NULL
            )
            ",
            [],
        )?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

impl Storage for SqliteStorage {
    fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let connection = self.connection.lock().unwrap();

        let mut statement = connection.prepare(
            "SELECT value FROM state WHERE key = ?1"
        )?;

        let result = statement.query_row(
            params![key],
            |row| row.get::<_, Vec<u8>>(0),
        );

        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn set(
        &self,
        key: &[u8],
        value: Vec<u8>,
    ) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "
            INSERT INTO state (key, value)
            VALUES (?1, ?2)
            ON CONFLICT(key)
            DO UPDATE SET value = excluded.value
            ",
            params![key, value],
        )?;

        Ok(())
    }

    fn remove(&self, key: &[u8]) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "DELETE FROM state WHERE key = ?1",
            params![key],
        )?;

        Ok(())
    }

    fn commit(&self) -> anyhow::Result<()> {
        // Dependendo de como você estruturar
        // as transações SQLite.
        Ok(())
    }
}