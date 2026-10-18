use std::{
    collections::HashMap,
    sync::RwLock,
};

use anyhow::Result;

use crate::ports::Storage;

/// [`Storage`] backed by an in-process [`HashMap`], guarded by an
/// [`RwLock`] so concurrent `get`s don't block each other.
///
/// Holds nothing on disk — state is lost when the process exits. Meant for
/// tests and examples; use [`crate::adapters::SqliteStorage`] for anything
/// that needs to persist.
///
/// # Example
///
/// ```
/// use ledger_me_sdk::adapters::InMemoryStorage;
/// use ledger_me_sdk::ports::Storage;
///
/// let storage = InMemoryStorage::new();
/// storage.set(b"key", b"value".to_vec())?;
/// assert_eq!(storage.get(b"key")?, Some(b"value".to_vec()));
/// # Ok::<(), anyhow::Error>(())
/// ```
pub struct InMemoryStorage {
    data: RwLock<HashMap<Vec<u8>, Vec<u8>>>,
}

impl InMemoryStorage {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl Storage for InMemoryStorage {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let data = self.data.read().unwrap();

        Ok(data.get(key).cloned())
    }

    fn set(&self, key: &[u8], value: Vec<u8>) -> Result<()> {
        let mut data = self.data.write().unwrap();

        data.insert(key.to_vec(), value);

        Ok(())
    }

    fn remove(&self, key: &[u8]) -> Result<()> {
        let mut data = self.data.write().unwrap();

        data.remove(key);

        Ok(())
    }

    fn commit(&self) -> Result<()> {
        Ok(())
    }
}