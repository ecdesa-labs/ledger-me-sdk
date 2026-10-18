# ADR 0001: `CallExecution` with one instance per execution

🌐 **English** · [Português](<0001 [ptBR] - CallExecution com uma instancia por execucao.md>)

- **Status:** Accepted
- **Date:** 2026-09-22
- **Code at the time:** commit `f13ae7f`

## Context

The [ROADMAP](../ROADMAP.md) plans a `CallExecution` actor to coordinate the
call tree of each top-level `Call`: root span, events, cross-pallet calls
with `call_chain` and, later, rollback.

The question was cost. A blockchain may receive thousands of executions per
second, and there were two ways to have this actor:

1. **One instance per execution:** for each top-level `Call`, the `Runtime`
   creates a new `CallExecution` (`.start()`, new `Addr`). Cross-pallet calls reuse
   that same instance, and it dies at the end.
2. **One fixed instance:** a single `CallExecution` created along with the
   `Runtime` and reused, with its state cleared between executions.

Since the `Runtime` is **serial** (one top-level `Call` at a time), both
options have only one active execution at a time. The only differences are
the cost of creating the actor and the risk of state leaking from one
execution into the next.

## Measurement

Release-mode benchmark (a standalone crate, outside the repository), 200k iterations for actor
operations and 20k for `Call`s, after a warmup. Environment: WSL2 (kernel
`6.18.33.2-microsoft-standard-WSL2`), 16 cores, `rustc 1.98.1`. Numbers
varied quite a bit between runs; the table shows the observed range.

| Operation | Cost per operation |
|---|---|
| Create a new actor (`Context`), send a message and await | ~0.95–1.14 µs |
| Send a message to an existing actor | ~0.60 µs |
| Hop to the `SyncArbiter(1)` thread, no WASM | ~60 µs |
| WASM only: create the `Store`, instantiate the module, run `sum` | ~56–82 µs |
| WASM only, with `InstancePre` | ~57–67 µs |
| `Call` straight to `Addr<Pallet>` | ~234 µs |
| **Full `Call`** (`Runtime` → `Pallet`) | **~227–275 µs** (≈ 3.6–4.4k/s) |

## Decision

**One `CallExecution` instance per top-level `Call` execution.** Cross-pallet
calls use the instance of the execution in progress.

- Creating the actor costs ~0.4–0.5 µs more than reusing one, about **0.2%**
  of a `Call`'s cost. Not a bottleneck.
- A fixed instance would save only that, but would require clearing state
  between executions. In a blockchain, state leaking from one transaction
  into another is a serious bug. With one instance per execution, the state
  dies with it and that class of bug can't happen.

## Consequences

- Throughput today is bounded by the latency of one `Call` (the `Runtime` is
  serial): about **4k `Call`s/s**. `CallExecution` doesn't change that number
  in any meaningful way.
- The real cost is elsewhere: the thread hop and WASM instantiation on every
  call, plus ~100 µs still unexplained. Tracked in
  [TODO.md, Performance section](../TODO.md#performance).
- Each cross-pallet call will cost **two extra thread hops**
  (A → `CallExecution` → B → back). That's far heavier than creating the
  actor, and is also in the TODO.
- These numbers are from WSL2, where waking a thread tends to be slower.
  Re-measure on native Linux before optimizing anything.
