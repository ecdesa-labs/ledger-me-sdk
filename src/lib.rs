//! SDK for running "pallets" — units of state-transition logic compiled to
//! WASM — inside a sandboxed [`wasmtime`] runtime.
//!
//! A pallet is built with [`PalletBuilder`], which compiles its WASM and
//! wires it to a [`ports::Storage`] backend and any host functions the
//! guest imports (exposed to guest code via `#[`[`host_fn`]`]`). The
//! resulting [`Pallet`] is driven by [`Call`]s, each producing a
//! [`CallResponse`]. Multiple pallets are addressed by name through the
//! [`Runtime`] actor.
//!
//! # Limits
//!
//! A pallet can't hang or take down the node, and every limit is
//! deterministic, so all nodes reach the same outcome (ADR 0002). The SDK
//! owns the wasmtime engine to guarantee that:
//!
//! - **Fuel:** each [`Call`] carries a [`fuel_limit`](Call::fuel_limit)
//!   (default [`Call::DEFAULT_FUEL_LIMIT`]); running out fails it with
//!   [`RuntimeError::OutOfFuel`], and [`CallResponse::fuel_used`] reports
//!   what was used. Most WASM instructions cost 1; **a host function call
//!   costs 1, whatever it does** (see
//!   [`PalletBuilder::with_host_functions`]).
//! - **Memory:** each pallet has a limit
//!   ([`PalletBuilder::with_memory_bytes`], default
//!   [`Pallet::DEFAULT_MEMORY_BYTES`]); growing past it fails the call
//!   with [`RuntimeError::OutOfMemory`].
//!
//! # Tracing
//!
//! The SDK emits spans through [`tracing`] but never installs a subscriber;
//! that's up to the program using it. Every call opens `call.execution`
//! (in the [`Runtime`]) and, under it, `pallet.call` (in the [`Pallet`]),
//! both with `pallet` and `function` fields. They hang under whatever span
//! the [`Call`] was created in: [`Call::new`] captures it in the call's
//! [`CallContext`], which travels with the call across mailboxes and
//! threads.
//!
//! Each call ends with one event in `call.execution`: `call succeeded`
//! (`DEBUG`, with `elapsed`, `output_bytes`, `events` and `fuel_used`) or `call failed`
//! (`WARN`, with `elapsed` and `error`). `examples/runtime_example` shows a
//! development setup (`tracing-subscriber`'s pretty format, `RUST_LOG`
//! filter).
//!
//! [`ports`] defines the storage traits pallets depend on (in hexagonal
//! architecture terms, the "ports"); [`adapters`] provides concrete
//! implementations of them (the "adapters") — in-memory for tests, SQLite
//! for persistence.
mod errors;
mod call;
mod call_execution;
mod engine;
pub mod ports;
pub mod adapters;
mod pallet;
mod runtime;
pub mod abi;

pub use call::{Call, CallContext, CallResponse};
pub use pallet::{Pallet, PalletBuilder, PalletContext, Set, Unset, call_bytes_host_fn};
pub use runtime::Runtime;
pub use errors::RuntimeError;
pub use ledger_me_sdk_macros::host_fn;
pub use wasmtime;
pub use actix;
