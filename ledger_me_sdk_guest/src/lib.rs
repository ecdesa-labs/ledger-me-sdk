//! Runtime support for pallet *guests* (the `#![no_std]` WASM crate a
//! contract author writes), paired with the `#[`[`call`]`]`/`#[`[`import`]`]`
//! macros that generate the FFI boilerplate around it.
//!
//! Depending on this crate gives a guest, for free:
//! - a `#[panic_handler]` (required by `#![no_std]`, otherwise every pallet
//!   has to define its own);
//! - the `alloc` export every pallet must have (see "How a `Call` runs"
//!   on [`Pallet`](https://docs.rs/ledger_me_sdk/latest/ledger_me_sdk/struct.Pallet.html#how-a-call-runs));
//! - [`read_input`]/[`write_output`], the raw memory plumbing `#[call]` and
//!   `#[import]`-generated code calls into.
//!
//! A guest crate still needs its own `Cargo.toml` dependency on this crate
//! (path or version) plus `extern crate alloc;` for `Vec`/`String`/etc.
#![no_std]

pub extern crate alloc;

pub use borsh;
pub use ledger_me_sdk_macros::{call, import};

use alloc::vec::Vec;

/// Packs a `(ptr, len)` pair into a single `i64`, `ptr` in the high 32 bits
/// and `len` in the low 32 bits.
///
/// Duplicated from `ledger_me_sdk::abi::pack_ptr_len` rather than shared,
/// since this crate is `#![no_std]` and can't depend on the host crate
/// (which pulls in `wasmtime`, `std`, etc.) — the format itself is a fixed,
/// tiny wire contract unlikely to ever change.
pub fn pack_ptr_len(ptr: u32, len: u32) -> i64 {
    ((ptr as i64) << 32) | len as i64
}

/// Inverse of [`pack_ptr_len`].
pub fn unpack_ptr_len(value: i64) -> (u32, u32) {
    ((value >> 32) as u32, value as u32)
}

/// Turns a guest panic into a WASM trap (`unreachable`), which the host gets
/// back as an `Err` for that one call; the pallet and the runtime keep
/// serving the next calls.
///
/// Never a `loop {}`: without fuel, the host has no way to stop it, so the
/// pallet's thread and the (serial) runtime would hang forever.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    #[cfg(target_arch = "wasm32")]
    core::arch::wasm32::unreachable();

    // Only reached when the crate is checked for a native target.
    #[cfg(not(target_arch = "wasm32"))]
    loop {}
}

// Backs both Rust's own allocations (`Vec`, `String`, ... via
// `#[global_allocator]`) and the WASM-visible `alloc` export the host calls
// directly — same heap, so the two can't hand out overlapping regions.
//
// `talc`'s wasm allocator grows the module's linear memory (`memory.grow`)
// on demand instead of the fixed-size bump pointer this used to be, which
// would silently write out of bounds once a call allocated past the
// module's initial memory size.
#[cfg(all(not(target_feature = "atomics"), target_family = "wasm"))]
#[global_allocator]
static ALLOCATOR: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();

/// The `alloc` export every pallet module must have (see "How a `Call`
/// runs" in `Pallet`'s docs) — the host calls this to get a scratch buffer
/// in guest memory before writing a call's input into it.
///
/// Named `guest_alloc` at the Rust level (`alloc` is already taken by the
/// `alloc` crate re-export above) but exported to WASM under the required
/// name via `#[unsafe(export_name)]`. Forwards to the global allocator
/// declared above.
///
/// Traps instead of returning a null pointer when there's no memory left:
/// the host would write the call's input at address 0, over the guest's
/// stack. The host already makes growing past the memory limit trap; this
/// is the second guard.
#[unsafe(export_name = "alloc")]
pub extern "C" fn guest_alloc(size: u32) -> u32 {
    // Zero bytes (e.g. a call with empty input) needs no memory, and asking
    // the allocator for 0 bytes is undefined behavior.
    if size == 0 {
        return core::ptr::NonNull::<u8>::dangling().as_ptr() as u32;
    }

    let ptr = unsafe { alloc::alloc::alloc(core::alloc::Layout::from_size_align_unchecked(size as usize, 1)) };

    if ptr.is_null() {
        panic!("guest_alloc: out of memory");
    }

    ptr as u32
}

/// Reads `len` bytes starting at `ptr` out of the guest's own linear memory.
///
/// Used both to read a `#[call]`'s input (`ptr`/`len` come from the host,
/// as described in `Pallet`'s docs) and to read a host import's return value (`ptr`/
/// `len` come from unpacking the import's result, written into guest memory
/// by `call_bytes_host_fn` on the host side).
///
/// # Safety
///
/// `ptr`/`len` must describe a region of the guest's memory that is valid
/// and already initialized — true for both cases above, never for
/// arbitrary caller-supplied values.
pub unsafe fn read_input(ptr: u32, len: u32) -> &'static [u8] {
    unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) }
}

/// Copies `bytes` into a freshly [`alloc`]'d region of guest memory and
/// packs the result as `(ptr, len)` — the return value a `#[call]`-exported
/// function hands back to the host.
pub fn write_output(bytes: Vec<u8>) -> i64 {
    let ptr = guest_alloc(bytes.len() as u32);

    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
    }

    pack_ptr_len(ptr, bytes.len() as u32)
}
