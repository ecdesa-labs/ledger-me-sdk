mod logging;

use std::sync::Arc;

use ledger_me_sdk::actix::Actor;
use ledger_me_sdk::adapters::InMemoryStorage;
use ledger_me_sdk::{Call, PalletBuilder, Runtime, RuntimeError};
use borsh::BorshSerialize;
use sha2::{Digest, Sha256};
use tracing::Instrument;

/// Mirrors the guest's `SumArgs` (examples/pallet_example/src/lib.rs) field
#[derive(BorshSerialize)]
struct SumArgs {
    a: u32,
    b: u32,
}

/// Registered below as the WASM guest's `crypto::sha256` import.
///
/// Costs the guest 1 fuel per call (plus a fixed ~217 for the `alloc` that
/// holds the result), however big the input: fuel only counts WASM
/// instructions. Offering it as a host function instead of leaving it to
/// the pallet in WASM is the chain maintainers' call (ADR 0002).
#[ledger_me_sdk::host_fn]
fn calc_hash(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[actix_rt::main]
async fn main() -> anyhow::Result<()> {
    logging::init()?;

    // The SDK compiles the WASM with its own deterministic engine (fuel on).
    // The guest's memory is capped at 16 MiB by default; `with_memory_bytes`
    // would change it.
    let example_pallet = PalletBuilder::new()
        .with_wasm(include_bytes!(concat!(env!("OUT_DIR"), "/pallet_example.wasm")))
        .with_storage(Arc::new(InMemoryStorage::new()))
        .with_host_functions(|linker| {
            linker.func_wrap("crypto", "sha256", calc_hash)?;
            Ok(())
        })?
        .build()?
        .start();

    let runtime = Runtime::new()
        .with_pallet("pallet_example", example_pallet)
        .start();

    let input = b"hello, ledger_me_sdk!";
    // `Call::new` captures the current span, so the SDK's spans for this
    // call show up under `hash_request`, and every event in it (the SDK's
    // and the one below) shares the same trace ID.
    async {
        let response = runtime.send(Call::new("pallet_example", "hash", input.to_vec())).await??;
        tracing::info!(input = %String::from_utf8_lossy(input), sha256 = %hex(&response.data), "hashed");

        anyhow::Ok(())
    }
    .instrument(tracing::info_span!("hash_request"))
    .await?;

    let args = SumArgs { a: 3, b: 4 };
    let call = Call::new("pallet_example", "sum", borsh::to_vec(&args)?);
    let response = runtime.send(call).await??;
    let sum: u32 = borsh::from_slice(&response.data)?;

    tracing::info!(a = args.a, b = args.b, sum, fuel_used = response.fuel_used, "summed");
    assert_eq!(sum, 7);

    // Every `Call` carries a fuel limit (`Call::new` sets 100 million).
    // Too little for the work fails that call with `OutOfFuel`, the same on
    // every node; a guest stuck in a loop ends the same way.
    let starved = Call::new("pallet_example", "sum", borsh::to_vec(&args)?).with_fuel_limit(100);
    let error = runtime.send(starved).await?.unwrap_err();
    assert!(matches!(error.downcast_ref(), Some(RuntimeError::OutOfFuel { .. })));
    tracing::info!(%error, "sum with too little fuel");

    // 3 bytes aren't a `SumArgs` (8 bytes), so the guest panics while
    // decoding. The panic becomes a trap: this call fails and the runtime
    // keeps serving the next one instead of hanging.
    let bad_call = Call::new("pallet_example", "sum", vec![1, 2, 3]);
    assert!(runtime.send(bad_call).await?.is_err());

    let call = Call::new("pallet_example", "sum", borsh::to_vec(&args)?);
    let sum: u32 = borsh::from_slice(&runtime.send(call).await??.data)?;
    assert_eq!(sum, 7);
    tracing::info!("still serving calls after a guest panic");

    Ok(())
}
