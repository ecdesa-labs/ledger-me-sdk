use anyhow::Result;

/// A key-value store a [`crate::Pallet`] reads and writes its state
/// through.
///
/// Implementations are handed to [`crate::PalletBuilder::with_storage`] as
/// `Arc<dyn Storage>`, shared across every call the pallet executes (see
/// [`crate::PalletContext`]). [`crate::adapters`] provides an in-memory and
/// a SQLite-backed implementation.
///
/// `Send + Sync` because each [`crate::Pallet`] runs on its own thread (see
/// [`crate::Pallet::start`]), and the storage goes there with it.
pub trait Storage: Send + Sync {
    /// Reads the value stored under `key`, or `None` if it isn't set.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    /// Sets `key` to `value`, overwriting any existing value.
    fn set(&self, key: &[u8], value: Vec<u8>) -> Result<()>;

    /// Removes `key`, if present.
    fn remove(&self, key: &[u8]) -> Result<()>;

    /// Flushes any buffered writes to the underlying backend. A no-op for
    /// backends that write through immediately.
    fn commit(&self) -> Result<()>;
}
