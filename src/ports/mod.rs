//! Traits pallets depend on for persistence, without committing to a
//! specific backend — the "ports" in hexagonal architecture terms.
//! Concrete backends live in [`crate::adapters`].
mod storage;
mod trie_db;

pub use storage::Storage;
pub use trie_db::TrieDB;
