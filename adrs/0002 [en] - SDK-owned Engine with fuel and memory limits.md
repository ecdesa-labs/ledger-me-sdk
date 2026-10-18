# ADR 0002: SDK-owned `Engine` with fuel and memory limits

🌐 **English** · [Português](<0002 [ptBR] - Engine do SDK com fuel e limite de memoria.md>)

- **Status:** Accepted
- **Date:** 2026-09-23
- **Code at the time:** commit `5325994`

## Context

A pallet whose WASM never finishes blocks its `Pallet` thread forever and,
since the `Runtime` is **serial**, every later `Call` to every pallet: the
whole chain stops.

Until commit `5325994`, a panic inside the guest did exactly that: the
guest's panic handler was `loop {}`. That commit made it trap
(`unreachable`), so a panic now fails only its own `Call`. But a guest can
still loop forever on its own (a bug, or bad faith), and nothing stops it.
It can also grow its memory up to the 4 GiB wasm32 limit on every `Call`
and take the node down.

Any limit has to be **deterministic**: every node must reach the same
result for the same `Call`, or their states diverge.

Wasmtime offers two ways to interrupt a guest:

1. **Fuel:** counts executed WASM instructions (most cost 1) and traps when
   it runs out. Deterministic. Only WASM is counted: **a host function
   call costs 1 fuel** (the `call` instruction), however much work the
   host does.
2. **Epoch interruption:** an external thread advances a clock and the
   guest traps past a deadline. Cheaper, but based on wall-clock time: a
   slow node could trap where a fast one doesn't.

Fuel is enabled on the `Engine` (`Config::consume_fuel`), and today the SDK
user creates the `Engine` and compiles the `Module`. Wasmtime's `Engine`
has no public getters, so the SDK can't check whether a given `Engine` has
fuel, or any of the other settings that affect determinism (NaN
canonicalization, relaxed SIMD, threads).

## Measurement

Release-mode benchmark (a standalone crate, outside the repository),
wasmtime 48.0.2. It measures only the WASM part of a `Call` (what
`Pallet::handle` does: new `Store`, instantiate, `alloc`, write the input,
call, read the output), since fuel doesn't touch the actor path. Warmup of
10% of the iterations. Two `Engine` configurations: default, and the SDK's
(fuel, NaN canonicalization, no relaxed SIMD, no threads, 16 MiB
`StoreLimits`). Five runs each, plus three runs with **only** fuel enabled
to isolate its cost. Environment: WSL2 (kernel
`6.18.33.2-microsoft-standard-WSL2`), 16 cores, `rustc 1.98.1`.

| Case | Default | SDK config | Difference |
|---|---|---|---|
| `pallet_example::sum` (20k iterations) | ~57–74 µs | ~58–79 µs | none measurable |
| `pallet_example::hash`, sha256 in a host function (20k) | ~59–79 µs | ~61–72 µs | none measurable |
| Rust guest, sha256 **inside WASM**, chained 1000× (500) | ~522–555 µs | ~589–657 µs | ~+10–25% |
| Rust guest, sha256 **inside WASM** of 64 KiB (2000) | ~441–581 µs | ~474–532 µs | ~0–15% |
| WAT loop, 14 instructions × 1M iterations (300) | ~606–833 µs | ~917–996 µs | ~+20–60% (worst case) |
| WAT load/store loop × 1M iterations (300) | ~634–849 µs | ~981–1,101 µs | ~+30–65% (worst case) |

- With only fuel enabled, the loops cost the same as the full SDK config:
  **the overhead is all fuel's**. NaN canonicalization costs nothing
  without floats, and the memory limiter only acts on `memory.grow`.
- Fuel used was **identical in every run**: `sum` 2,106; `hash` 2,740;
  in-WASM sha256 chained 1000× 7,425,605; 64 KiB 5,640,685.
- With fuel on, this machine runs about **11–19 million fuel per ms**
  (11–12 on the real sha256, 15–19 on the loops).
- A Rust guest starts with 17 pages of memory (1.06 MiB: 1 MiB of stack
  plus data).
- **Host function calls**, measured separately (1,000 calls in a WAT loop,
  minus the empty loop): **exactly 1 fuel per call**, both for a host
  function that does nothing and for one that computes 10,000 sha256s.
  A `#[host_fn]` adds a **fixed** ~217: the guest's `alloc` it calls to
  return the result. That doesn't depend on the work done either.

## Decision

**The SDK owns the `Engine`, with a deterministic configuration and fuel.
The fuel limit travels in the `Call`, and each pallet has a memory limit.**

1. **SDK-owned `Engine`.** One per process, created by the SDK with fuel
   on, NaN canonicalization on, relaxed SIMD off, threads off and
   multi-memory off (see item 3). The
   `PalletBuilder` takes the WASM bytes and compiles the `Module` itself.
   `with_engine`/`with_module` leave the API, and `build()` returns
   `Result` (compiling can fail).
2. **Fuel comes from the `Call`.** A public field `call.fuel_limit`, next
   to `pallet`, `function` and `data`: the limit is part of the request,
   like a transaction's gas limit. `Call::new` sets a default of **100
   million** (~5–9 ms of CPU on the machine above, ~47k times a `sum`),
   and `with_fuel_limit(n)` overrides it. The `Pallet` pours it into the
   `Store` **before** instantiating (a module's `start` function also
   uses fuel). Running out is a trap, and the SDK returns it as
   `RuntimeError::OutOfFuel`, an `Err` for that `Call` only.
   `CallResponse` gains `fuel_used`, which the `call succeeded` event also
   logs. Later the limit will come from the transaction, and with
   cross-pallet calls the `CallExecution` will use it as the budget of the
   whole call tree.
3. **Memory limit per pallet.** An optional `with_memory_bytes(n)` on the
   builder, default **16 MiB** (~15× a Rust guest's initial memory),
   applied on every `Call`'s `Store` (through `StoreLimits` at first; see
   the [update](#update-2026-09-23-typed-errors-for-memory-and-table-limits)
   below). `build()`
   fails if the module's initial memory is already above the limit.
   Unlike fuel, memory belongs to each instance (in A → B, A and B have
   separate memories), so a per-pallet limit is coherent. Three details
   close the gaps around it:
   - **Growing past the limit traps** (`trap_on_grow_failure`) instead of
     returning -1. Otherwise the guest's `alloc` export (the door through
     which the host asks the guest's allocator for room to write a
     `Call`'s input or a host function's result) returns address 0, and
     the host would write over the guest's stack. The guest SDK's `alloc`
     also traps on a null pointer, as a second guard.
   - **One memory per module:** wasmtime applies the limit to each memory
     separately, and a module may declare several (multi-memory, on by
     default in wasmtime), multiplying the limit. Rust guests use exactly
     one, so multi-memory is turned off on the `Engine` and a module with
     more than one memory fails in `build()`.
   - **Table limit:** WASM keeps functions called through a pointer
     (`dyn Trait`, `fn` pointers) in a numbered list, the "table", which
     the compiler creates on its own. A module can grow it at runtime
     (`table.grow`), and each entry uses node memory outside the 16 MiB.
     The `Store` caps it at **100,000 entries** per table; the guests
     measured here have no table at all.
4. Sending a `Call` straight to an `Addr<Pallet>` keeps working (the
   `Call` always carries a fuel limit). The `Pallet` rustdoc warns that
   this skips the `CallExecution`: no `call.execution` span and, later, no
   shared fuel budget or rollback.

Rejected:

- **Epoch interruption:** not deterministic.
- **`Engine` provided by the user:** it can't be validated, and every node
  operator would have to get five settings right by hand.
- **Optional fuel:** one node forgetting to enable it is exactly the
  divergence we want to rule out.
- **Fuel limit per pallet:** in A → B it's unclear whose limit applies. The
  budget belongs to the top-level `Call`.
- **A second `Engine` without fuel for trusted pallets:** not now. The
  measured cost doesn't justify it (see Consequences).

## Consequences

- **Cost:** none measurable on today's `Call`s, where instantiating
  dominates. About +10–25% on pure WASM execution in real code, and up to
  +50–65% in the worst case (tiny loops). It only applies to WASM
  execution, not to the thread hop, instantiation or the ~100 µs still
  unexplained (ADR 0001), which weigh far more in a `Call`.
- **A host function costs 1 fuel per call, whatever it does.** Fuel only
  counts WASM instructions: the host's work (a sha256 of 1 MB, a storage
  read) costs 0, and a `#[host_fn]` only adds the guest's fixed `alloc`
  to return the result. Host functions run natively, so they're also
  much faster than the same code in WASM.
- **Offering host functions is the chain maintainers' decision.** Whoever
  assembles the chain chooses, function by function, between:
  - **providing it as a host function:** fast, and 1 fuel per call, so
    fuel doesn't limit it. The chain maintainers become responsible for
    its cost (bounding the input size, or charging for it: the `Caller`
    can read and set fuel, see the [TODO](../TODO.md));
  - **leaving it to the pallet maintainers** to implement in WASM: slower,
    but every instruction is metered by fuel.

  The SDK doesn't impose either: it's a trade-off between performance and
  metering that belongs to each chain. `pallet_example` uses the first
  option with `crypto::sha256`.
- **Consensus parameters:** the default fuel limit, each pallet's memory
  limit, the table limit and the `Engine` configuration must be the same
  on every node.
  Since they're set by whoever writes the chain (every node runs the same
  binary), documenting it is enough. The fuel count depends on the `.wasm`
  and on wasmtime's cost per instruction: treat a wasmtime upgrade as a
  possible consensus change until it's checked.
- **Fuel ↔ time depends on the machine.** The count is deterministic; how
  long 100 million takes is not. The default is a safety net against
  infinite loops, not a price.
- **Stack depth is still not deterministic:** deep recursion already traps
  (it doesn't hang), but wasmtime's stack limit is in native bytes, so one
  node may trap where another doesn't. Recorded in the TODO.
- **Public API changes:** `PalletBuilder` loses `with_engine`/`with_module`
  and gains `with_wasm` and `with_memory_bytes`; `build()` returns
  `Result`; `Call` gains `fuel_limit` and `with_fuel_limit`;
  `CallResponse` gains `fuel_used`; `RuntimeError` gains `OutOfFuel`. The README examples change with it.
- **No rollback yet:** a `Call` that runs out of fuel or memory after
  writing to storage keeps those writes, same as any other failure. That's
  phase 5 of the [ROADMAP](../ROADMAP.md).
- If a real pallet shows fuel's cost matters, the way out is an extra
  fuel-free `Engine` for trusted pallets, with its own ADR.

## Update (2026-09-23): typed errors for memory and table limits

Running out of fuel had its own error, `RuntimeError::OutOfFuel`, but
running out of memory or table entries was only a generic `Err`.
Wasmtime's `StoreLimits` reports those as text only ("forcing trap when
growing memory to N bytes"), unlike fuel, which comes as a typed
`Trap::OutOfFuel`.

**Decision:** replace `StoreLimits` with the SDK's own `ResourceLimiter`,
a struct implementing wasmtime's trait of the same name. When a growth would pass a limit, it returns a typed
error, which wasmtime turns into a trap and hands back intact:

- `RuntimeError::OutOfMemory { memory_bytes }` for the guest's memory;
- `RuntimeError::TableLimitExceeded { max_elements }` for a table.

A growth that the limiter allows but that fails anyway (past the module's
own declared maximum) also traps, so the "trap, never -1" rule from
item 3 still holds, now without `trap_on_grow_failure`.

**Rejected:**

- **Matching wasmtime's error text:** a wasmtime upgrade that rewords it
  would silently break the detection.
- **Reusing `OutOfMemory` for tables:** the guest's memory didn't run out;
  the log would point at the wrong limit.

Nothing else changes: limits, defaults and determinism are the same.

