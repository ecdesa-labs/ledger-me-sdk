use std::marker::PhantomData;

use anyhow::Result;
use wasmtime::{Caller, ExternType, Linker, Module};

use crate::{Pallet, PalletContext, ports::Storage};

/// Reads the guest's `(ptr, len)` argument from WASM linear memory, calls
/// `f` with those bytes, writes the result back via the guest's `alloc`
/// export, and returns it packed as `(ptr, len)` via [`crate::abi::pack_ptr_len`].
///
/// This is the marshaling logic the [`macro@crate::host_fn`] attribute macro
/// generates a call to; most consumers annotate a plain function with
/// `#[host_fn]` instead of calling this directly.
///
/// Fuel: the call itself costs the guest 1, and `f` costs nothing however
/// much work it does (fuel only counts WASM instructions). The only other
/// fuel used is the guest's `alloc` run to hold the result: a fixed ~217
/// with the guest SDK's allocator. See
/// [`PalletBuilder::with_host_functions`].
pub fn call_bytes_host_fn(
    mut caller: Caller<'_, PalletContext>,
    ptr: u32,
    len: u32,
    f: impl FnOnce(&[u8]) -> Vec<u8>,
) -> wasmtime::Result<i64> {
    let memory = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("WASM memory not found"))?;

    let mut input = vec![0u8; len as usize];
    memory.read(&caller, ptr as usize, &mut input)?;

    let output = f(&input);

    let alloc = caller
        .get_export("alloc")
        .and_then(|export| export.into_func())
        .ok_or_else(|| wasmtime::Error::msg("alloc export not found"))?
        .typed::<u32, u32>(&caller)?;

    let output_ptr = alloc.call(&mut caller, output.len() as u32)?;
    memory.write(&mut caller, output_ptr as usize, &output)?;

    Ok(crate::abi::pack_ptr_len(output_ptr, output.len() as u32))
}

/// Typestate marker: the corresponding [`PalletBuilder`] field hasn't been
/// set yet.
pub struct Unset;

/// Typestate marker: the corresponding [`PalletBuilder`] field has been set.
pub struct Set;

/// Builder for assembling the pieces required to run a WASM pallet.
///
/// It collects the pallet's WASM bytes, the [`Storage`] backend, an
/// optional memory limit and, through
/// [`with_host_functions`](Self::with_host_functions), any host functions
/// the pallet needs to import (e.g. `crypto::sha256`).
///
/// The wasmtime [`Engine`](wasmtime::Engine) isn't set here: the SDK owns
/// it, so every node compiles and runs pallets with the same deterministic
/// configuration (fuel on, NaN canonicalization, no relaxed SIMD, threads
/// or multi-memory; see ADR 0002).
///
/// Host functions are registered via a callback instead of being hardcoded
/// in this crate, so each SDK consumer can expose exactly the imports their
/// pallet needs.
///
/// # Typestate
///
/// `PalletBuilder<W, S>` encodes "have the WASM and the storage been set?"
/// as two type parameters, each either [`Set`] or [`Unset`]:
///
/// - [`PalletBuilder::new`] returns `PalletBuilder<Unset, Unset>`.
/// - `with_wasm`/`with_storage` each flip their *own* type parameter from
///   `Unset` to `Set` and leave the other one untouched, so they can be
///   called in any order.
/// - [`with_host_functions`](Self::with_host_functions) and
///   [`with_memory_bytes`](Self::with_memory_bytes) work in any state.
/// - [`build`](Self::build) is only implemented for
///   `PalletBuilder<Set, Set>`.
///
/// Calling `build()` on a builder that's missing the WASM or the storage
/// is therefore "no method named `build` found for struct
/// `PalletBuilder<...>`" at compile time, not a panic or an `Err` found
/// while testing (or worse, in production).
///
/// `W` and `S` don't correspond to any actual data — [`Set`] and [`Unset`]
/// are zero-sized marker types that exist only to be type parameters. Since
/// none of `PalletBuilder`'s real fields have type `W` or `S`, the compiler
/// would otherwise reject them as unused type parameters (E0392); the
/// private `_marker: PhantomData<(W, S)>` field is what tells it these
/// parameters are used, at the type level only. `PhantomData` itself is
/// zero-sized, so this costs nothing at runtime.
///
/// # Example
///
/// ```
/// use std::sync::Arc;
/// use sha2::{Digest, Sha256};
/// use ledger_me_sdk::PalletBuilder;
/// use ledger_me_sdk::adapters::InMemoryStorage;
///
/// // Minimal valid (empty) WASM module, for illustration purposes.
/// const EMPTY_MODULE: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
///
/// // A plain named function can be passed directly to `func_wrap`,
/// // no closure needed.
/// fn add_one(x: i32) -> i32 {
///     x + 1
/// }
///
/// // `#[host_fn]` turns a plain `&[u8] -> Vec<u8>` function into one of
/// // those pluggable functions too, without writing the `Caller`/memory
/// // marshaling by hand.
/// #[ledger_me_sdk::host_fn]
/// fn calc_hash(data: &[u8]) -> Vec<u8> {
///     Sha256::digest(data).to_vec()
/// }
///
/// let pallet = PalletBuilder::new()
///     .with_wasm(EMPTY_MODULE)
///     .with_storage(Arc::new(InMemoryStorage::new()))
///     .with_host_functions(|linker| {
///         linker.func_wrap("math", "add_one", add_one)?;
///         linker.func_wrap("crypto", "sha256", calc_hash)?;
///
///         Ok(())
///     })?
///     .build()?;
/// # let _ = pallet;
/// # Ok::<(), anyhow::Error>(())
/// ```
pub struct PalletBuilder<W = Unset, S = Unset> {
    wasm: Option<Vec<u8>>,
    linker: Option<Linker<PalletContext>>,
    storage: Option<std::sync::Arc<dyn Storage>>,
    memory_bytes: usize,
    _marker: PhantomData<(W, S)>,
}

impl PalletBuilder<Unset, Unset> {
    /// Creates an empty builder with no WASM, storage or host functions
    /// configured yet, and the default memory limit
    /// ([`Pallet::DEFAULT_MEMORY_BYTES`]).
    ///
    /// # Example
    ///
    /// ```
    /// use ledger_me_sdk::PalletBuilder;
    ///
    /// let builder = PalletBuilder::new();
    /// # let _ = builder;
    /// ```
    pub fn new() -> Self {
        Self {
            wasm: None,
            linker: None,
            storage: None,
            memory_bytes: Pallet::DEFAULT_MEMORY_BYTES,
            _marker: PhantomData,
        }
    }
}

impl<S> PalletBuilder<Unset, S> {
    /// Sets the pallet's compiled WASM module: the `.wasm` bytes (or WAT
    /// text). It's compiled by [`build`](PalletBuilder::build), with the
    /// SDK's engine.
    ///
    /// # Example
    ///
    /// ```
    /// use ledger_me_sdk::PalletBuilder;
    ///
    /// const EMPTY_MODULE: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    ///
    /// let builder = PalletBuilder::new().with_wasm(EMPTY_MODULE);
    /// # let _ = builder;
    /// ```
    pub fn with_wasm(self, wasm: impl Into<Vec<u8>>) -> PalletBuilder<Set, S> {
        PalletBuilder {
            wasm: Some(wasm.into()),
            linker: self.linker,
            storage: self.storage,
            memory_bytes: self.memory_bytes,
            _marker: PhantomData,
        }
    }
}

impl<W> PalletBuilder<W, Unset> {
    /// Sets the [`Storage`] backend the pallet will read/write state from.
    ///
    /// # Example
    ///
    /// ```
    /// use std::sync::Arc;
    /// use ledger_me_sdk::PalletBuilder;
    /// use ledger_me_sdk::adapters::InMemoryStorage;
    ///
    /// let storage = Arc::new(InMemoryStorage::new());
    /// let builder = PalletBuilder::new().with_storage(storage);
    /// # let _ = builder;
    /// ```
    pub fn with_storage(self, storage: std::sync::Arc<dyn Storage>) -> PalletBuilder<W, Set> {
        PalletBuilder {
            wasm: self.wasm,
            linker: self.linker,
            storage: Some(storage),
            memory_bytes: self.memory_bytes,
            _marker: PhantomData,
        }
    }
}

impl<W, S> PalletBuilder<W, S> {
    /// Sets the most memory, in bytes, the pallet's guest may grow to
    /// during one call. Optional: defaults to
    /// [`Pallet::DEFAULT_MEMORY_BYTES`] (16 MiB).
    ///
    /// Growing past it fails that call with
    /// [`RuntimeError::OutOfMemory`](crate::RuntimeError::OutOfMemory).
    /// [`build`](PalletBuilder::build) fails if the module's *initial*
    /// memory is already bigger (a Rust guest starts at about 1 MiB).
    ///
    /// Part of consensus: every node must use the same value for the same
    /// pallet, or a call could fail on some nodes only.
    ///
    /// # Example
    ///
    /// ```
    /// use ledger_me_sdk::PalletBuilder;
    ///
    /// let builder = PalletBuilder::new().with_memory_bytes(32 << 20); // 32 MiB
    /// # let _ = builder;
    /// ```
    pub fn with_memory_bytes(mut self, memory_bytes: usize) -> Self {
        self.memory_bytes = memory_bytes;
        self
    }

    /// Registers host functions on the pallet's [`Linker`] through a callback.
    ///
    /// The linker is created on first call and reused, so
    /// `with_host_functions` can be called multiple times to register
    /// different groups of imports (e.g. once per module, such as `crypto`,
    /// `state`, etc.).
    ///
    /// # Fuel: a host function costs 1 per call
    ///
    /// Fuel only counts WASM instructions. Calling a host function costs
    /// the guest **1 fuel, whatever the host function does**: a sha256 of
    /// 1 MB costs the same as one of 1 byte. (A `#[host_fn]` also runs the
    /// guest's `alloc` to hold its result, a fixed ~217 fuel.) Host
    /// functions also run natively, so they're much faster than the same
    /// code in WASM.
    ///
    /// So **offering a host function is the chain maintainers' decision**,
    /// function by function:
    ///
    /// - offer it as a host function: fast and not limited by fuel, and
    ///   bounding its cost (input size, or charging fuel through
    ///   [`Caller::get_fuel`]/[`Caller::set_fuel`]) is on the chain
    ///   maintainers;
    /// - or leave it to the pallet maintainers to implement in WASM:
    ///   slower, but every instruction is metered by fuel.
    ///
    /// # Errors
    ///
    /// Returns an error if `register` itself fails (e.g. a name collision
    /// in the linker).
    ///
    /// A registered function can come in any of the three shapes
    /// `Linker::func_wrap` accepts:
    ///
    /// - a plain named function (`fn`), passed by name;
    /// - a function annotated with `#[`[`host_fn`](crate::host_fn)`]`, which
    ///   turns a plain `&[u8] -> Vec<u8>` function into one, also passed by
    ///   name;
    /// - an inline closure, for anything that doesn't fit the two shapes
    ///   above (e.g. it needs a [`Caller`] to read/write WASM memory itself).
    ///
    /// # Example
    ///
    /// ```
    /// use sha2::{Digest, Sha256};
    /// use ledger_me_sdk::PalletBuilder;
    ///
    /// // Plain named function.
    /// fn add_one(x: i32) -> i32 {
    ///     x + 1
    /// }
    ///
    /// // `&[u8] -> Vec<u8>` function turned into a host function by the macro.
    /// #[ledger_me_sdk::host_fn]
    /// fn calc_hash(data: &[u8]) -> Vec<u8> {
    ///     Sha256::digest(data).to_vec()
    /// }
    ///
    /// let builder = PalletBuilder::new()
    ///     .with_host_functions(|linker| {
    ///         linker.func_wrap("math", "add_one", add_one)?;
    ///         linker.func_wrap("crypto", "sha256", calc_hash)?;
    ///
    ///         // Inline closure.
    ///         linker.func_wrap("math", "double", |x: i32| -> i32 { x * 2 })?;
    ///
    ///         Ok(())
    ///     })?;
    /// # let _ = builder;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn with_host_functions<F>(mut self, register: F) -> Result<Self>
    where
        F: FnOnce(&mut Linker<PalletContext>) -> Result<()>,
    {
        let mut linker = self
            .linker
            .take()
            .unwrap_or_else(|| Linker::new(crate::engine::engine()));
        register(&mut linker)?;
        self.linker = Some(linker);

        Ok(self)
    }
}

impl PalletBuilder<Set, Set> {
    /// Compiles the WASM with the SDK's engine and builds the [`Pallet`]
    /// from it, the storage, the memory limit and the host functions.
    ///
    /// If [`with_host_functions`](Self::with_host_functions) was never
    /// called, the pallet gets an empty linker (fine for a module with no
    /// imports).
    ///
    /// The result is plain data, not yet running — call
    /// [`Pallet::start`] on it to get an `Addr<Pallet>` you can send
    /// [`crate::Call`]s to.
    ///
    /// # Errors
    ///
    /// If the WASM doesn't compile (including a module with more than one
    /// memory: the SDK's engine has multi-memory off), or if the module's
    /// initial `memory` is already bigger than the memory limit (every call
    /// would fail).
    ///
    /// # Example
    ///
    /// ```
    /// use std::sync::Arc;
    /// use ledger_me_sdk::PalletBuilder;
    /// use ledger_me_sdk::adapters::InMemoryStorage;
    ///
    /// const EMPTY_MODULE: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    ///
    /// let pallet = PalletBuilder::new()
    ///     .with_wasm(EMPTY_MODULE)
    ///     .with_storage(Arc::new(InMemoryStorage::new()))
    ///     .build()?;
    /// # let _ = pallet;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn build(self) -> Result<Pallet> {
        let wasm = self
            .wasm
            .expect("PalletBuilder<Set, Set> guarantees the WASM is set");

        let storage = self
            .storage
            .expect("PalletBuilder<Set, Set> guarantees the storage is set");

        let engine = crate::engine::engine();
        let module = Module::new(engine, &wasm)?;

        if let Some(ExternType::Memory(memory)) = module.get_export("memory") {
            let initial_bytes = memory.minimum().saturating_mul(memory.page_size());

            anyhow::ensure!(
                initial_bytes <= self.memory_bytes as u64,
                "the module's initial memory ({initial_bytes} bytes) is bigger than the pallet's memory limit ({} bytes)",
                self.memory_bytes,
            );
        }

        let linker = self.linker.unwrap_or_else(|| Linker::new(engine));

        Ok(Pallet::new(module, linker, storage, self.memory_bytes))
    }
}
