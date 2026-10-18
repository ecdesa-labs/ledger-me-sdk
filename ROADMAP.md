# Roadmap

🌐 **English** · [Português](ROADMAP.pt-BR.md)

What we'll build, in order. Project rule: **"Everything in Actix, nothing
against Actix, nothing outside Actix."** Decisions still open live in
[TODO.md](TODO.md).

## ✅ 1. Tracing (done)

- The SDK depends only on `tracing` (the facade). It emits spans and events
  but **never installs a subscriber**. That's up to whoever assembles the
  node.
- A `CallContext` travels inside every `Call` (public field `call.context`).
  For now it only holds `parent_span`, which `Call::new` fills with
  `Span::current()`, so nothing changes for SDK users.
- Spans, both with `pallet` and `function` fields:
  - `call.execution` in the `CallExecution` (see phase 2): root of each call;
  - `pallet.call` in the `Pallet`: `span.enter()` on the `SyncArbiter` thread.
  - The planned `runtime.route` span was dropped: `call.execution` already
    is each call's root, so a `Runtime` span would only repeat it.
- Each call ends with one event: `call succeeded` (`DEBUG`: `elapsed`,
  `output_bytes`, `events`) or `call failed` (`WARN`: `elapsed`, `error`).
- `runtime_example` logs for development with `tracing-subscriber`'s
  `pretty()` format (span tree, file and line per event). `RUST_LOG`
  overrides the default filter `info,ledger_me_sdk=debug`.
- `runtime_example` also installs a `tracing-opentelemetry` layer and shows
  each event's `trace_id` (one per call tree) in the terminal
  (`src/logging.rs`). The SDK didn't change: trace IDs are the subscriber's
  job. Nothing is exported yet.
- Test: `tests/tracing_spans.rs` checks the span tree
  (caller → `call.execution` → `pallet.call`).

## ✅ 2. `CallExecution`: one actor per call execution (done)

Decided in [ADR 0001](<adrs/0001 [en] - CallExecution with one instance per execution.md>):
a new instance per top-level `Call`, not one fixed instance that gets reused.

- The `Runtime` stays **serial**: one top-level `Call` at a time. For each
  `Call` it creates a `CallExecution`, which coordinates that `Call`'s whole
  call tree and dies at the end.
- In this phase, `CallExecution` only holds the root span and collects
  events. It stops on its own once the `Runtime` drops its `Addr`.
- `CallExecution`'s handlers use `ResponseActFuture` (not serial), so its
  mailbox stays free while it waits on a pallet.

## ✅ 3. SDK-owned `Engine` and execution limits (done)

Decided in [ADR 0002](<adrs/0002 [en] - SDK-owned Engine with fuel and memory limits.md>):
a guest must not be able to hang or take down the node, and every limit
must be deterministic.

- The SDK creates the `Engine` (one per process) with fuel, NaN
  canonicalization, no relaxed SIMD, no threads and no multi-memory. `PalletBuilder` takes
  the WASM bytes (`with_wasm`) instead of `with_engine`/`with_module`, and
  `build()` returns `Result`.
- The fuel limit travels in the `Call` (`call.fuel_limit`): `Call::new`
  sets 100 million, `with_fuel_limit(n)` overrides it. The `Pallet` pours
  it into the `Store` before instantiating. Running out returns
  `RuntimeError::OutOfFuel`, for that `Call` only. `CallResponse` gains
  `fuel_used`, also logged by `call succeeded`.
- Memory limit per pallet: optional `with_memory_bytes(n)`, default 16 MiB,
  through the SDK's own `ResourceLimiter`. `build()`
  fails if the module's initial memory is already above it. Growing past
  it fails the call with `RuntimeError::OutOfMemory` (a trap, never -1),
  and
  the guest SDK's `alloc` traps on a null pointer. Tables capped at 100,000
  entries (`RuntimeError::TableLimitExceeded`).
- The `Pallet` rustdoc warns that sending straight to `Addr<Pallet>` skips
  the `CallExecution`.
- The rustdoc of `#[host_fn]` and `with_host_functions` states that **a host
  function costs 1 fuel per call**, whatever it does, and that offering it
  (fast, unmetered) or leaving it to the pallets in WASM (slower, metered)
  is the chain maintainers' decision.
- `RuntimeError::OutOfFuel` carries the `fuel_limit` that ran out, so the
  log says which limit was hit.
- The guest SDK's `alloc` also stopped asking the allocator for 0 bytes
  (undefined behavior; a call with empty input did it).
- `runtime_example` shows `fuel_used` and a `Call` failing with
  `OutOfFuel` (`with_fuel_limit(100)`).
- Tests (`tests/execution_limits.rs`): a WAT infinite loop, a `memory.grow`
  and a `table.grow` past the limits all come back as `Err`, and the next
  `Call` still answers; `build()` rejects a module whose initial memory is
  above the limit, and one with two memories.

## 4. Cross-pallet calls through `CallExecution`

- `CallContext` gains the `Addr<CallExecution>` (moved here from phase 2:
  nothing uses it before cross-pallet calls exist).
- Guest: `#[import(pallet = "name")]` emits a WASM module with a reserved
  prefix (e.g. `pallet:name`). In `build()`, every import with that prefix is
  linked to a forwarder that sends the `Call` to the `CallExecution`.
- `CallExecution` resolves the target through the routing table the
  `Runtime` lends it (an `Arc<HashMap<name, Addr<Pallet>>>`).
- `call_chain` (stack of active pallets): `CallExecution` rejects, with an
  error, a target already on the stack instead of deadlocking (reentrancy
  guard).
- A pallet **never** sends a `Call` to `Addr<Runtime>`: the `Runtime` is
  waiting on that same pallet, so it would deadlock.
- Optional: the `Runtime` checks in `started()` that every imported pallet is
  registered.
- The `Call`'s fuel limit becomes the budget of the whole call tree: the
  `CallExecution` hands B what A has left and charges what B used (ADR
  0002).

## 5. Pending changes and rollback (atomicity)

- `CallExecution` owns the transaction's pending changes (an
  `Arc<Mutex<...>>` lent to pallets through the context): write everything if
  it succeeds, discard everything if any level fails.
- No lock contention: within a transaction only one pallet runs at a time.

## Later (when there's a real need)

- **Distributed tracing:** at the network or RPC edge, propagate with the
  W3C `traceparent` standard via `tracing-opentelemetry` and export over OTLP
  (Jaeger, Tempo). Only makes sense once there's a network. Trace IDs
  themselves already exist in `runtime_example`. No SDK change needed:
  - **Receiving** an upstream trace: `TraceContextPropagator::new().extract(&headers)`,
    then `span.set_parent(upstream)` on a span of ours, **before** anything
    runs inside it, and create the `Call` inside that span. Checked by hand:
    the call logged the upstream `trace_id`.
  - **Sending** it on: `span.context()` + `TraceContextPropagator::inject_context(...)`
    produce the outgoing `traceparent`.
  - `traceparent` carries `trace_id` **and** the caller's `span_id`. With a
    bare trace ID, build the context by hand (`SpanContext::new`), losing the
    link to the caller's span.
  - `traceparent` travels in the message's envelope/metadata, never in
    signed or hashed content (determinism, see TODO).
- **Guest logging:** a `log::emit(level, msg)` host function and a `log!`
  macro in `ledger_me_sdk_guest`, becoming `tracing::event!` inside
  `pallet.call`.
- **Events as messages:** broadcast to `Recipient<PalletEvent>` once
  something consumes them (RPC, indexer).
- **Node actors:** block execution, mempool and consensus, talking to
  `Addr<Runtime>`.
