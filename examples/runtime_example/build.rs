use std::{env, fs, path::PathBuf, process::Command};

/// Compiles a sibling guest pallet crate (`../{name}`) to
/// `wasm32-unknown-unknown` and copies the result to `$OUT_DIR/{name}.wasm`,
/// so `main.rs` can `include_bytes!` it. Living in `OUT_DIR`, the artifact
/// is never tracked by git and is rebuilt if `target/` is wiped.
///
/// Mirrors how a real pallet would ship: a standalone crate (own
/// `rust-toolchain.toml`, own `[workspace]`) compiled independently of the
/// host, with only the resulting artifact consumed here. Assumes the crate
/// directory name, package name and produced artifact all match `name`.
fn compile_guest_pallet(name: &str) -> PathBuf {
    let guest_dir = PathBuf::from("..").join(name);

    println!("cargo:rerun-if-changed={}", guest_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", guest_dir.join("Cargo.toml").display());

    let status = Command::new("cargo")
        .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
        .current_dir(&guest_dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to run cargo for {name}: {e}"));

    assert!(status.success(), "{name} build failed");

    let wasm_path = guest_dir.join(format!(
        "target/wasm32-unknown-unknown/release/{name}.wasm"
    ));

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join(format!("{name}.wasm"));
    fs::copy(&wasm_path, &out_path)
        .unwrap_or_else(|e| panic!("failed to copy {}: {e}", wasm_path.display()));

    out_path
}

fn main() {
    // Every guest pallet is built on top of the guest SDK, so a change there
    // (e.g. its panic handler) must rebuild them too.
    println!("cargo:rerun-if-changed=../../ledger_me_sdk_guest/src");
    println!("cargo:rerun-if-changed=../../ledger_me_sdk_guest/Cargo.toml");

    compile_guest_pallet("pallet_example");
}
