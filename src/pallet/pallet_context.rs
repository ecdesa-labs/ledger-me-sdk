use std::sync::Arc;

use crate::{Pallet, RuntimeError, ports::Storage};

/// The `wasmtime::Store` data for a single [`crate::Call`] run by a
/// [`crate::Pallet`].
///
/// A fresh `PalletContext` is created per call (see "How a `Call` runs" on
/// [`Pallet`](crate::Pallet)), so `events` always starts
/// empty and only ever accumulates events from that one call; `storage` is
/// cloned from the pallet's shared backend, so writes persist across calls.
/// Host functions reach both through `Caller::data()`/`data_mut()`.
pub struct PalletContext {
    pub storage: Arc<dyn Storage>,
    pub events: Vec<Vec<u8>>,
    /// Memory and table limits of this call's `Store`, set by the
    /// [`Pallet`](crate::Pallet) (wasmtime reads them from the store data).
    pub(crate) limits: ResourceLimiter,
}

impl PalletContext {
    /// Creates a context wrapping `storage`, with no events recorded yet.
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            storage,
            events: Vec::new(),
            limits: ResourceLimiter {
                memory_bytes: Pallet::DEFAULT_MEMORY_BYTES,
            },
        }
    }
}

/// Stops a guest from growing its memory or a table past the limits, with
/// a typed [`RuntimeError`] so the caller can tell which limit was hit
/// (wasmtime's own `StoreLimits` only reports it as text). Implements
/// wasmtime's trait of the same name.
///
/// Every failed growth is a trap, never `-1`: a `-1` would make the
/// guest's `alloc` hand back address 0, and the host would write over the
/// guest's stack.
pub(crate) struct ResourceLimiter {
    pub(crate) memory_bytes: usize,
}

impl wasmtime::ResourceLimiter for ResourceLimiter {
    fn memory_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        if desired > self.memory_bytes {
            return Err(RuntimeError::OutOfMemory {
                memory_bytes: self.memory_bytes,
            }
            .into());
        }

        Ok(true)
    }

    fn table_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        if desired > Pallet::MAX_TABLE_ELEMENTS {
            return Err(RuntimeError::TableLimitExceeded {
                max_elements: Pallet::MAX_TABLE_ELEMENTS,
            }
            .into());
        }

        Ok(true)
    }

    // Growth allowed above can still fail (past the module's own declared
    // maximum): trap then too, instead of returning -1.
    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        Err(error)
    }

    fn table_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        Err(error)
    }
}
