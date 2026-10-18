use crate::ports::TrieDB;
use rusqlite::{params, Connection};
use std::sync::Mutex;

/// [`TrieDB`] backed by a single-table SQLite database, storing trie nodes
/// keyed by hash.
///
/// As with [`crate::adapters::SqliteStorage`], the connection is guarded by
/// a [`Mutex`] since `rusqlite::Connection` isn't `Sync`.
pub struct SqliteTrieDb {
    connection: Mutex<Connection>,
}

impl SqliteTrieDb {
    /// Opens (or creates) the SQLite database at `path` and ensures its
    /// `trie_nodes` table exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection can't be opened or the table
    /// can't be created.
    pub fn new(path: &str) -> rusqlite::Result<Self> {
        let connection = Connection::open(path)?;

        connection.execute(
            "
            CREATE TABLE IF NOT EXISTS trie_nodes (
                hash  BLOB PRIMARY KEY,
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

impl TrieDB for SqliteTrieDb {
    type Error = rusqlite::Error;

    fn get(
        &self,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        let connection = self.connection.lock().unwrap();

        let mut statement = connection.prepare(
            "SELECT value FROM trie_nodes WHERE hash = ?1"
        )?;

        let result = statement.query_row(
            params![key],
            |row| row.get::<_, Vec<u8>>(0),
        );

        match result {
            Ok(value) => Ok(Some(value)),

            Err(rusqlite::Error::QueryReturnedNoRows) => {
                Ok(None)
            }

            Err(error) => Err(error),
        }
    }

    fn insert(
        &self,
        key: &[u8],
        value: Vec<u8>,
    ) -> Result<(), Self::Error> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "
            INSERT OR REPLACE INTO trie_nodes(hash, value)
            VALUES (?1, ?2)
            ",
            params![key, value],
        )?;

        Ok(())
    }

    fn remove(
        &self,
        key: &[u8],
    ) -> Result<(), Self::Error> {
        let connection = self.connection.lock().unwrap();

        connection.execute(
            "DELETE FROM trie_nodes WHERE hash = ?1",
            params![key],
        )?;

        Ok(())
    }

    fn flush(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}