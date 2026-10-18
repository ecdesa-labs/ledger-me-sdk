use thiserror::Error;

/// Errors returned by [`crate::Runtime`] and [`crate::Pallet`].
#[derive(Error, Debug)]
pub enum RuntimeError {
    /// The [`crate::Runtime`] got a [`crate::Call`] addressed to a pallet
    /// name that was never registered with [`crate::Runtime::with_pallet`].
    #[error("pallet not found: {0}")]
    PalletNotFound(String),

    /// The [`crate::Call`] used up all of its [`fuel_limit`](crate::Call::fuel_limit)
    /// before finishing. Deterministic: every node runs out at the same
    /// instruction.
    #[error("out of fuel (limit: {fuel_limit})")]
    OutOfFuel { fuel_limit: u64 },

    /// The guest tried to grow its memory past the pallet's
    /// [`memory limit`](crate::PalletBuilder::with_memory_bytes).
    #[error("out of memory (limit: {memory_bytes} bytes)")]
    OutOfMemory { memory_bytes: usize },

    /// The guest tried to grow a WASM table past
    /// [`Pallet::MAX_TABLE_ELEMENTS`](crate::Pallet::MAX_TABLE_ELEMENTS)
    /// entries. A table is the list of functions WASM calls through a
    /// pointer; each entry uses node memory outside the guest's memory
    /// limit.
    #[error("table limit exceeded (limit: {max_elements} entries)")]
    TableLimitExceeded { max_elements: usize },
}
