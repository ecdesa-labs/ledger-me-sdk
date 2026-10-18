//! A pallet can't hang or take down the node (ADR 0002): running out of
//! fuel or memory fails only that call, and the next one still answers.

use std::sync::Arc;

use ledger_me_sdk::actix::{Actor, Addr};
use ledger_me_sdk::adapters::InMemoryStorage;
use ledger_me_sdk::{Call, Pallet, PalletBuilder, Runtime, RuntimeError};

/// `alloc` grows memory to fit each request (like a real allocator would),
/// `spin` never returns, `grow` asks for 32 MiB more memory, `grow_table`
/// asks for more table entries than the SDK allows, `noop` does nothing.
const GUEST: &str = r#"
(module
  (memory (export "memory") 1)
  (table 0 funcref)
  (func (export "alloc") (param $size i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (i32.mul (memory.size) (i32.const 65536)))
    (drop (memory.grow (i32.add (i32.shr_u (local.get $size) (i32.const 16)) (i32.const 1))))
    (local.get $ptr))
  (func (export "spin") (param i32 i32) (result i64)
    (loop (br 0))
    i64.const 0)
  (func (export "grow") (param i32 i32) (result i64)
    (drop (memory.grow (i32.const 512)))
    i64.const 0)
  (func (export "grow_table") (param i32 i32) (result i64)
    (drop (table.grow (ref.null func) (i32.const 200000)))
    i64.const 0)
  (func (export "noop") (param i32 i32) (result i64) i64.const 0))
"#;

fn start_runtime() -> anyhow::Result<Addr<Runtime>> {
    let pallet = PalletBuilder::new()
        .with_wasm(GUEST)
        .with_storage(Arc::new(InMemoryStorage::new()))
        .build()?
        .start();

    Ok(Runtime::new().with_pallet("guest", pallet).start())
}

async fn call(runtime: &Addr<Runtime>, call: Call) -> anyhow::Result<u64> {
    Ok(runtime.send(call).await??.fuel_used)
}

#[actix_rt::test]
async fn infinite_loop_runs_out_of_fuel_and_the_next_call_answers() -> anyhow::Result<()> {
    let runtime = start_runtime()?;

    let error = call(&runtime, Call::new("guest", "spin", vec![])).await.unwrap_err();
    assert!(matches!(
        error.downcast_ref(),
        Some(RuntimeError::OutOfFuel { fuel_limit: Call::DEFAULT_FUEL_LIMIT })
    ));

    let fuel_used = call(&runtime, Call::new("guest", "noop", vec![])).await?;
    assert!(fuel_used > 0);

    Ok(())
}

#[actix_rt::test]
async fn fuel_limit_comes_from_the_call() -> anyhow::Result<()> {
    let runtime = start_runtime()?;

    let fuel_used = call(&runtime, Call::new("guest", "noop", vec![])).await?;
    let too_little = Call::new("guest", "noop", vec![]).with_fuel_limit(fuel_used - 1);

    let error = call(&runtime, too_little).await.unwrap_err();
    assert!(matches!(error.downcast_ref(), Some(RuntimeError::OutOfFuel { .. })));

    Ok(())
}

#[actix_rt::test]
async fn growing_past_the_limits_fails_the_call_and_the_next_call_answers() -> anyhow::Result<()> {
    let runtime = start_runtime()?;

    // 1 page + 512 pages = ~32 MiB, past the default 16 MiB.
    let memory_error = call(&runtime, Call::new("guest", "grow", vec![])).await.unwrap_err();
    assert!(matches!(
        memory_error.downcast_ref(),
        Some(RuntimeError::OutOfMemory { memory_bytes: Pallet::DEFAULT_MEMORY_BYTES })
    ));

    let table_error = call(&runtime, Call::new("guest", "grow_table", vec![])).await.unwrap_err();
    assert!(matches!(
        table_error.downcast_ref(),
        Some(RuntimeError::TableLimitExceeded { max_elements: Pallet::MAX_TABLE_ELEMENTS })
    ));

    call(&runtime, Call::new("guest", "noop", vec![])).await?;

    Ok(())
}

#[actix_rt::test]
async fn input_bigger_than_the_memory_limit_fails_the_call() -> anyhow::Result<()> {
    let runtime = start_runtime()?;

    // The host asks the guest's `alloc` for room, and the guest's memory
    // would have to grow past 16 MiB to hold it.
    let error = call(&runtime, Call::new("guest", "noop", vec![0; 20 << 20])).await.unwrap_err();
    assert!(matches!(error.downcast_ref(), Some(RuntimeError::OutOfMemory { .. })));

    call(&runtime, Call::new("guest", "noop", vec![])).await?;

    Ok(())
}

#[test]
fn build_rejects_initial_memory_above_the_limit() {
    let result = PalletBuilder::new()
        .with_wasm(GUEST) // 1 page = 64 KiB initial memory
        .with_storage(Arc::new(InMemoryStorage::new()))
        .with_memory_bytes(32 << 10)
        .build();

    assert!(result.is_err());
}

#[test]
fn build_rejects_more_than_one_memory() {
    let result = PalletBuilder::new()
        .with_wasm(r#"(module (memory 1) (memory 1))"#)
        .with_storage(Arc::new(InMemoryStorage::new()))
        .build();

    assert!(result.is_err());
}
