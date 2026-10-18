//! Example pallet guest for `examples/runtime_example`: calls the host's
//! `crypto::sha256` import and returns its result, plus a typed `sum` call
//! demonstrating the `borsh`-encoded (non-raw-bytes) path through `#[call]`.
#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use ledger_me_sdk_guest::borsh::{BorshDeserialize, BorshSerialize};
use ledger_me_sdk_guest::{call, import};

#[import(module = "crypto")]
extern "C" {
    fn sha256(data: &[u8]) -> Vec<u8>;
}

#[call]
fn hash(input: Vec<u8>) -> Vec<u8> {
    sha256(&input)
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct SumArgs {
    pub a: u32,
    pub b: u32,
}

#[call]
fn sum(args: SumArgs) -> u32 {
    args.a + args.b
}
