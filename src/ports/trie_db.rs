/// Port for a trie node store, backing an [`eth_trie`] Merkle-Patricia
/// trie. Re-exported as-is from `eth_trie` rather than wrapped, since its
/// `get`/`insert`/`remove`/`flush` shape is already exactly what a trie
/// node store needs. [`crate::adapters::SqliteTrieDb`] is the SQLite
/// implementation.
pub use eth_trie::DB as TrieDB;
