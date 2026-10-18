//! Wire format for passing a `(ptr, len)` pair across the host/guest WASM
//! boundary as a single value.
//!
//! A WASM function can only return one value, so a host function that needs
//! to hand back a byte slice (a pointer into the guest's linear memory plus
//! its length) packs both into a single `i64`: `ptr` in the high 32 bits,
//! `len` in the low 32 bits. [`crate::call_bytes_host_fn`] packs the result
//! on the way out; a [`crate::Pallet`] unpacks a `#[call]`'s result the
//! same way on the way back in.

/// Packs a `(ptr, len)` pair into a single `i64`, `ptr` in the high 32 bits
/// and `len` in the low 32 bits.
///
/// # Example
///
/// ```
/// use ledger_me_sdk::abi::{pack_ptr_len, unpack_ptr_len};
///
/// let packed = pack_ptr_len(0x1000, 42);
/// assert_eq!(unpack_ptr_len(packed), (0x1000, 42));
/// ```
pub fn pack_ptr_len(ptr: u32, len: u32) -> i64 {
    ((ptr as i64) << 32) | len as i64
}

/// Inverse of [`pack_ptr_len`]: splits a packed `i64` back into its
/// `(ptr, len)` pair.
///
/// # Example
///
/// ```
/// use ledger_me_sdk::abi::unpack_ptr_len;
///
/// assert_eq!(unpack_ptr_len(0x0000_1000_0000_002a), (0x1000, 42));
/// ```
pub fn unpack_ptr_len(value: i64) -> (u32, u32) {
    (
        (value >> 32) as u32,
        value as u32,
    )
}
