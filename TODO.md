# TODO: what we still need to resolve

🌐 **English** · [Português](TODO.pt-BR.md)

Open problems and decisions. What's already decided and only needs building
lives in [ROADMAP.md](ROADMAP.md).

Vocabulary: **serial** = the actor processes one message at a time, start to
finish. **Atomic** = all-or-nothing in storage. Don't mix the two.

Marker: **🤔 undecided** = I don't know yet what I'll do; these are just
ideas, nothing decided.

## State and consistency

- [ ] **Rollback:** doesn't exist today. If a pallet fails midway, the writes
  made up to that point stay. Needs deciding:
  - the shape of the pending changes;
  - how they interact with `Storage::commit` (a no-op today; see
    `src/adapters/sqlite_storage.rs`);
  - whether reads inside the transaction see its own still-pending writes.
- [ ] **Long-term global ordering:** today the `Runtime` is serial and
  guarantees ordering. Once the block-execution actor exists, who should own
  that guarantee, it or the `Runtime`?
- [ ] **Reentrancy:** the plan is to reject every cycle (A→B→A). Need to
  confirm whether some legitimate case (e.g. a pallet calling itself) should
  be allowed.

## Tracing

- [ ] **Trace IDs never go into state, events or consensus.** They're random
  and would break determinism across nodes. How do we enforce this beyond
  documenting it?
- [ ] **Correlation by `tx_hash`:** the transaction hash as a span field
  (identical on every node). The SDK has no concept of a transaction yet.
- [x] **Where `CallContext` lives:** decided, public field `call.context`.
  Anyone building `Call { .. }` directly must fill it
  (`CallContext::from_current_span()`); `Call::new` does it for you.

## Robustness

- [ ] **Panic on a pallet's thread** (e.g. a Rust panic in a host
  function): the `SyncArbiter` doesn't restart it, and later calls get
  `MailboxError`. Do we want supervision? WASM traps, including a panic
  inside the guest (its panic handler traps with `unreachable`), already
  arrive as `Err` and don't kill the thread.
- [x] **Guest stuck in an infinite loop:** decided in
  [ADR 0002](<adrs/0002 [en] - SDK-owned Engine with fuel and memory limits.md>)
  (fuel, SDK-owned `Engine`, memory limit per pallet) and implemented in
  ROADMAP phase 3.
- [ ] **Stack depth isn't deterministic:** deep recursion traps (it doesn't
  hang), but wasmtime's limit is in native bytes, so one node may trap
  where another doesn't. Substrate solved it by instrumenting the `.wasm`
  to count depth. Is it worth it?
- [ ] **A host function costs 1 fuel per call, whatever it does** (a
  `sha256` of 100 MB included). Offering host functions is the chain
  maintainers' decision (ADR 0002); open question: should the SDK help
  them charge for it (the `Caller` can read and set fuel), e.g.
  `#[host_fn]` charging per byte? That's already real "gas".
- [ ] **"A pallet never sends a `Call` to the `Runtime`":** today this is
  only documented. Can the type system prevent it (the pallet having no
  access to `Addr<Runtime>`)?

## Performance

Numbers and context in [ADR 0001](<adrs/0001 [en] - CallExecution with one instance per execution.md>).
Today a full `Call` costs ~230–270 µs, and with a serial `Runtime` that's
about **4k `Call`s/s**. Creating a `CallExecution` per execution costs ~0.2%
of that and isn't a bottleneck.

- [ ] 🤔 **undecided: a thread hop on every `Call`** (~60 µs on WSL2). Idea:
  send a whole block of `Call`s in one message, paying the hop once per block.
- [ ] 🤔 **undecided: instantiating the WASM on every call** (~56–82 µs).
  Idea: wasmtime's pooling allocator (`PoolingAllocationConfig`).
  `InstancePre` alone barely changed anything.
- [ ] 🤔 **undecided: ~100 µs unexplained.** The path through the pallet
  actor costs more than the thread hop and the WASM combined. Investigate
  before optimizing anything else.
- [ ] 🤔 **undecided: cross-pallet calls cost two extra thread hops**
  (A → `CallExecution` → B → back). Far heavier than creating the actor.
  Doesn't block `CallExecution`, but must be tackled when cross-pallet calls
  land.
- [ ] 🤔 **undecided: parallel execution** of transactions that don't touch
  the same data. Much later.
- [ ] Re-measure on **native Linux** before optimizing: on WSL2 the numbers
  varied a lot between runs.
