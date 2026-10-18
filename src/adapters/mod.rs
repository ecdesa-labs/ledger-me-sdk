//! Concrete implementations of the traits in [`crate::ports`] — the
//! "adapters" in hexagonal architecture terms.
mod in_memory_storage;
mod sqlite_storage;
mod sqlite_trie_db;

pub use in_memory_storage::InMemoryStorage;
pub use sqlite_storage::SqliteStorage;
pub use sqlite_trie_db::SqliteTrieDb;
