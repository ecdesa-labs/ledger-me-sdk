mod pallet_context;
mod pallet_builder;

use actix::prelude::{Actor, Addr, Handler, Message, SyncArbiter, SyncContext};
use anyhow::Result;
use wasmtime::{Linker, Module, Store, Trap};

use crate::{
    RuntimeError,
    call::{Call, CallResponse},
    ports::Storage,
};

pub use pallet_context::PalletContext;
pub use pallet_builder::{PalletBuilder, Set, Unset, call_bytes_host_fn};

/// A unit of state-transition logic compiled to WASM, running as its own
/// actix actor with its own mailbox.
///
/// [`PalletBuilder::build`] compiles one from its WASM bytes, storage,
/// memory limit and host functions — plain data, not yet an actor.
/// [`Pallet::start`] turns it into a running actor; from then on it's
/// driven only by [`Call`] messages (`Addr::send`/`do_send`), same as any
/// other actix actor.
///
/// A pallet is a [`SyncContext`] actor: executing WASM is blocking CPU
/// work, which must never run on an actix event loop (it would stall every
/// other actor sharing that loop, including the [`crate::Runtime`]).
///
/// **Send your [`Call`]s through the [`crate::Runtime`].** Sending one
/// straight to an `Addr<Pallet>` works (the `Call` carries its own fuel
/// limit), but skips the `CallExecution` the `Runtime` creates for it: no
/// `call.execution` span nor `call succeeded`/`call failed` event, and
/// later no fuel budget shared across cross-pallet calls nor rollback.
///
/// # How a `Call` runs
///
/// Each call gets its own [`wasmtime::Store`] and [`PalletContext`] — the
/// module is re-instantiated every time, so pallets have no memory that
/// persists between calls except through `storage` (cloned from the shared
/// backend, so writes there *do* persist). Steps:
///
/// 1. Fill the `Store` with [`Call::fuel_limit`] fuel and cap its memory at
///    the pallet's limit (see [`PalletBuilder::with_memory_bytes`]). Before
///    instantiating, since a module's `start` function uses fuel too.
/// 2. Instantiate the module against the pallet's [`Linker`].
/// 3. Look up its `memory` and `alloc` exports (required of every guest;
///    see [`call_bytes_host_fn`] for the host-function side of the same
///    contract).
/// 4. `alloc` a buffer in guest memory, write `call.data` into it.
/// 5. Call the export named `call.function` with that buffer's
///    `(ptr, len)`, and unpack its packed `(ptr, len)` result (see
///    [`crate::abi`]).
/// 6. Read the output bytes back out of guest memory.
///
/// Events recorded on the [`PalletContext`] during the call (by host
/// functions writing to `Caller::data_mut().events`) are returned
/// alongside the output and the fuel used in the [`CallResponse`].
///
/// # Limits
///
/// Both are deterministic, so every node gets the same outcome (ADR 0002):
///
/// - **Fuel:** every WASM instruction uses some (most use 1). Running out
///   fails the call with [`RuntimeError::OutOfFuel`]. A host function
///   call costs 1 fuel, however much work the host does: see
///   [`PalletBuilder::with_host_functions`].
/// - **Memory:** growing the guest's memory past the limit fails the call
///   with [`RuntimeError::OutOfMemory`], and growing a table past
///   [`Pallet::MAX_TABLE_ELEMENTS`] entries with
///   [`RuntimeError::TableLimitExceeded`].
///
/// The call fails (the `Err` in the reply to `Addr::send`) if it runs out
/// of fuel, if instantiation fails, if the module doesn't export
/// `memory`/`alloc`/`call.function`, if the guest traps (a guest panic is
/// a trap), or if any of the memory/call operations themselves fail.
#[derive(Clone)]
pub struct Pallet {
    module: Module,
    linker: Linker<PalletContext>,
    storage: std::sync::Arc<dyn Storage>,
    memory_bytes: usize,
}

impl Actor for Pallet {
    type Context = SyncContext<Self>;
}

impl Message for Call {
    type Result = Result<CallResponse>;
}

impl Handler<Call> for Pallet {
    type Result = Result<CallResponse>;

    /// Runs one [`Call`] to completion; see "How a `Call` runs" on
    /// [`Pallet`].
    fn handle(&mut self, call: Call, _ctx: &mut Self::Context) -> Self::Result {
        let span = tracing::info_span!(
            parent: &call.context.parent_span,
            "pallet.call",
            pallet = %call.pallet,
            function = %call.function,
        );
        let _entered = span.enter();

        self.run(&call).map_err(|error| {
            if let Some(Trap::OutOfFuel) = error.downcast_ref::<Trap>() {
                return RuntimeError::OutOfFuel {
                    fuel_limit: call.fuel_limit,
                }
                .into();
            }

            // `OutOfMemory`/`TableLimitExceeded`, raised by the SDK's `ResourceLimiter`.
            match error.downcast::<RuntimeError>() {
                Ok(limit_error) => limit_error.into(),
                Err(error) => error.into(),
            }
        })
    }
}

impl Pallet {
    /// Default for [`PalletBuilder::with_memory_bytes`]: 16 MiB, about 15×
    /// the initial memory of a Rust guest (1 MiB of stack plus data).
    pub const DEFAULT_MEMORY_BYTES: usize = 16 << 20;

    /// Most entries a WASM table may grow to. A table is the numbered list
    /// of functions WASM calls through a pointer (`dyn Trait`, `fn`
    /// pointers); the compiler creates it on its own, and each entry uses
    /// node memory outside the guest's memory limit. Rust guests use a few
    /// hundred at most.
    pub const MAX_TABLE_ELEMENTS: usize = 100_000;

    /// The WASM part of [`Handler::handle`], in wasmtime's own error type
    /// so an out-of-fuel trap can still be told apart.
    fn run(&self, call: &Call) -> wasmtime::Result<CallResponse> {
        let mut context = PalletContext::new(self.storage.clone());
        context.limits.memory_bytes = self.memory_bytes;

        let mut store = Store::new(crate::engine::engine(), context);
        store.limiter(|context| &mut context.limits);
        store.set_fuel(call.fuel_limit)?;

        let instance = self.linker.instantiate(&mut store, &self.module)?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| wasmtime::format_err!("WASM memory export not found"))?;

        let alloc = instance.get_typed_func::<u32, u32>(&mut store, "alloc")?;

        let wasm_call = instance.get_typed_func::<(u32, u32), i64>(&mut store, &call.function)?;

        let ptr = alloc.call(&mut store, call.data.len() as u32)?;

        memory.write(&mut store, ptr as usize, &call.data)?;

        let result = wasm_call.call(&mut store, (ptr, call.data.len() as u32))?;

        let (result_ptr, result_len) = crate::abi::unpack_ptr_len(result);

        let mut output = vec![0u8; result_len as usize];

        memory.read(&store, result_ptr as usize, &mut output)?;

        let events = store.data().events.clone();
        let fuel_used = call.fuel_limit - store.get_fuel()?;

        Ok(CallResponse::new(output, events, fuel_used))
    }

    /// Starts this pallet as an actor on its own dedicated thread and
    /// returns its address.
    ///
    /// Shortcut for `SyncArbiter::start(1, ...)`. Exactly **one** thread on
    /// purpose: the mailbox then handles [`Call`]s strictly one at a time,
    /// in arrival order, so state transitions stay deterministic — two
    /// threads racing on the same storage would not be.
    ///
    /// Must be called from inside a running actix `System`.
    pub fn start(self) -> Addr<Pallet> {
        SyncArbiter::start(1, move || self.clone())
    }

    /// Assembles a [`Pallet`] from its already-configured pieces.
    ///
    /// Crate-private: [`PalletBuilder`] enforces at compile time that the
    /// WASM and storage are set before a `Pallet` can be built, and
    /// validates the module, so it's the only sanctioned way to get one.
    pub(crate) fn new(module: Module, linker: Linker<PalletContext>, storage: std::sync::Arc<dyn Storage>, memory_bytes: usize) -> Self {
        Self {
            module,
            linker,
            storage,
            memory_bytes,
        }
    }
}
