# ledger_me_sdk

🌐 **English** · [Português](README.pt-BR.md)

`ledger_me_sdk` is a Rust SDK for building modular blockchains. Business
logic is written as **pallets**: Rust crates compiled to WebAssembly and
executed by the host in a sandboxed [wasmtime](https://wasmtime.dev)
runtime, with coordination handled by the [actix](https://actix.rs) actor
model.

> **Status:** early development. The public API may change between
> versions. See the [ROADMAP](ROADMAP.md) for what is implemented and what
> is planned.

## Motivation

Building a blockchain is still a job for a handful of specialists. This
crate exists to **democratize blockchain creation**: it offers a
**modular layer** so companies and developers can build their own chains
from ready-made pieces instead of starting from scratch.

The goal is for more people to look at blockchain technology with fresh
eyes and start building more blockchains that add real value to their
businesses.

## Design

- **Isolation.** Each pallet is a separate WASM module, instantiated anew
  for every call; state persists only through the storage backend. A trap
  or a panic inside a pallet fails that call only.
- **Deterministic execution.** The SDK owns the wasmtime engine and its
  configuration (fuel metering and NaN canonicalization enabled; relaxed
  SIMD, threads and multi-memory disabled), so the same call yields the
  same result on every node.
- **Execution limits.** Each call carries a fuel limit and each pallet has
  a memory limit. Exceeding either fails the call without affecting the
  runtime. See [Execution limits](#execution-limits).
- **Ordered execution.** The `Runtime` processes one top-level call at a
  time, and each pallet runs on its own dedicated thread.
- **Host functions.** Native functions are exposed to pallets with
  `#[host_fn]`. They run at native speed and cost the calling pallet 1
  fuel per call.
- **Guest macros.** `#[call]` and `#[import]` generate the WASM ABI glue
  and the Borsh encoding, so pallet functions are written as typed Rust
  functions.
- **Tracing.** Every call is a [`tracing`](https://docs.rs/tracing) span
  with `pallet`, `function`, `elapsed` and `fuel_used`. The SDK emits
  spans only; installing a subscriber is left to the application.

## Usage

A pallet (the guest) is compiled to WASM on its own. The host (your node)
loads it and sends it `Call`s.

### 1. Pallet (guest)

`Cargo.toml`:

```toml
[package]
name = "my_pallet"
version = "0.1.0"
edition = "2021"

[workspace]

[lib]
crate-type = ["cdylib"]

[dependencies]
ledger_me_sdk_guest = "0.1"
borsh = { version = "1", default-features = false, features = ["derive"] }

[profile.release]
opt-level = "z"
lto = true
panic = "abort"
```

`src/lib.rs`:

```rust
#![no_std]

// Rust's `alloc` crate (`Vec`, `String`). The allocator itself (talc) and
// the `alloc` export the host uses come from `ledger_me_sdk_guest`.
extern crate alloc;

use alloc::vec::Vec;
use ledger_me_sdk_guest::borsh::{BorshDeserialize, BorshSerialize};
use ledger_me_sdk_guest::{call, import};

// Functions the host exposes to this pallet.
#[import(module = "crypto")]
extern "C" {
    fn sha256(data: &[u8]) -> Vec<u8>;
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct SumArgs {
    pub a: u32,
    pub b: u32,
}

// `#[call]` decodes the input and encodes the output with Borsh.
#[call]
fn sum(args: SumArgs) -> u32 {
    args.a + args.b
}

// `Vec<u8>` in and out are passed through as raw bytes; the hashing
// itself runs on the host.
#[call]
fn hash(input: Vec<u8>) -> Vec<u8> {
    sha256(&input)
}
```

Build it:

```sh
cargo build --release --target wasm32-unknown-unknown
```

### 2. Runtime (host)

`Cargo.toml`:

```toml
[dependencies]
ledger_me_sdk = "0.1"
actix-rt = "2"
anyhow = "1"
borsh = { version = "1", features = ["derive"] }
sha2 = "0.11"
```

`src/main.rs`:

```rust
use std::sync::Arc;

use ledger_me_sdk::actix::Actor;
use ledger_me_sdk::adapters::InMemoryStorage;
use ledger_me_sdk::{Call, PalletBuilder, Runtime, RuntimeError};
use borsh::BorshSerialize;
use sha2::{Digest, Sha256};

#[derive(BorshSerialize)]
struct SumArgs {
    a: u32,
    b: u32,
}

/// Offered to the pallet as `crypto::sha256`. Runs natively and costs the
/// pallet 1 fuel per call, however big the input.
#[ledger_me_sdk::host_fn]
fn sha256(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

#[actix_rt::main]
async fn main() -> anyhow::Result<()> {
    let my_pallet = PalletBuilder::new()
        .with_wasm(include_bytes!("../my_pallet.wasm"))
        .with_storage(Arc::new(InMemoryStorage::new()))
        .with_memory_bytes(32 << 20) // optional: 16 MiB by default
        .with_host_functions(|linker| {
            linker.func_wrap("crypto", "sha256", sha256)?;
            Ok(())
        })?
        .build()?
        .start();

    let runtime = Runtime::new()
        .with_pallet("my_pallet", my_pallet)
        .start();

    // A typed call, with the fuel it used.
    let args = borsh::to_vec(&SumArgs { a: 3, b: 4 })?;
    let response = runtime.send(Call::new("my_pallet", "sum", args.clone())).await??;
    let sum: u32 = borsh::from_slice(&response.data)?;
    assert_eq!(sum, 7);
    println!("sum = {sum}, fuel used = {}", response.fuel_used);

    // A call that uses a host function.
    let response = runtime.send(Call::new("my_pallet", "hash", b"hello".to_vec())).await??;
    assert_eq!(response.data.len(), 32);

    // Not enough fuel: that call fails, deterministically, and the
    // runtime keeps serving the next ones.
    let starved = Call::new("my_pallet", "sum", args).with_fuel_limit(100);
    let error = runtime.send(starved).await?.unwrap_err();
    assert!(matches!(error.downcast_ref(), Some(RuntimeError::OutOfFuel { .. })));

    Ok(())
}
```

### Running the bundled example

[`examples/runtime_example`](examples/runtime_example) builds a pallet and
runs it with structured logs, including OpenTelemetry trace IDs:

```sh
cargo run -p runtime_example
```

> **Expected warnings:** the example makes two calls fail on purpose, and
> each one is logged as `WARN call failed`. One runs with too little fuel
> (`with_fuel_limit(100)`) and fails with `out of fuel`; the other sends
> `sum` 3 bytes instead of a `SumArgs`, so the guest panics while decoding.
> They show that a failing call affects only itself: the calls right after
> them are still answered.

Excerpt of the output:

```text
DEBUG call succeeded, elapsed: 1.2ms, output_bytes: 4, events: 0, fuel_used: 2121
  in call.execution with pallet: pallet_example, function: sum
INFO  summed, a: 3, b: 4, sum: 7, fuel_used: 2121
WARN  call failed, error: out of fuel (limit: 100)
WARN  call failed, error: error while executing at wasm backtrace: ... (guest panic)
INFO  still serving calls after a guest panic
```

## Execution limits

All limits are deterministic: every node reaches the same outcome for the
same call.

| Limit | Default | Configured with | When exceeded |
|---|---|---|---|
| Fuel per `Call` (≈ executed WASM instructions) | 100 million (~5–9 ms of CPU on the benchmark machine) | `Call::with_fuel_limit(n)` | the call fails with `RuntimeError::OutOfFuel` |
| Guest memory per pallet | 16 MiB | `PalletBuilder::with_memory_bytes(n)` | the call fails with `RuntimeError::OutOfMemory` |
| Guest panic | — | — | the call fails; the runtime keeps serving calls |

**Host functions and fuel.** Fuel counts WASM instructions only, so a host
function costs **1 fuel per call regardless of the work it does**. Whether
a function is offered as a host function (fast, not metered) or left to
pallet maintainers to implement in WASM (slower, metered) is a decision
for the **chain maintainers**, who are then responsible for bounding the
cost of what they expose. See
[ADR 0002](<adrs/0002 [en] - SDK-owned Engine with fuel and memory limits.md>).

## Roadmap

Planned next, in order:

1. **Cross-pallet calls:** pallets calling one another within a single
   execution, sharing one fuel budget, with reentrancy protection.
2. **Atomic executions:** pending storage writes are discarded if any
   pallet in the call tree fails.

Details in the [ROADMAP](ROADMAP.md).

## Architecture Decision Records

- 📁 [adrs/](adrs/)
  - [0001 - CallExecution with one instance per execution](<adrs/0001 [en] - CallExecution with one instance per execution.md>)
  - [0002 - SDK-owned Engine with fuel and memory limits](<adrs/0002 [en] - SDK-owned Engine with fuel and memory limits.md>)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
