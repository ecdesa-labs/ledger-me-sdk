use std::sync::OnceLock;

use wasmtime::{Config, Engine};

/// The one wasmtime [`Engine`] every [`crate::Pallet`] is compiled and run
/// with (see ADR 0002).
///
/// Owned by the SDK so no node can end up with a configuration that makes
/// the same `Call` behave differently from another node's:
///
/// - **fuel** on: a deterministic limit on executed WASM instructions;
/// - **NaN canonicalization** on: float NaNs get the same bits on any CPU;
/// - **relaxed SIMD** off: its results depend on the hardware;
/// - **threads** off: shared-memory threads aren't deterministic;
/// - **multi-memory** off: the memory limit applies per memory, so several
///   memories would multiply it. Rust guests use exactly one.
pub(crate) fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();

    ENGINE.get_or_init(|| {
        let mut config = Config::new();
        config
            .consume_fuel(true)
            .cranelift_nan_canonicalization(true)
            .wasm_relaxed_simd(false)
            .wasm_threads(false)
            .wasm_multi_memory(false);

        Engine::new(&config).expect("the SDK's wasmtime config is valid")
    })
}
