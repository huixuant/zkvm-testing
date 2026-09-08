#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // Lifted from the sha3-chain benchmark:
    //     fn sha3_chain(input: [u8; 32], num_iters: u32) -> [u8; 32]
    //
    // Byte layout of `input` (little-endian, total 33 bytes required):
    //   [0..32]  msg      : [u8; 32]  the 32-byte seed hashed by the chain
    //   [32]     iters_raw: u8        maps to num_iters = (iters_raw % 8) + 1
    //
    // num_iters is derived from one byte and clamped to 1..=8 so the keccak
    // chain stays within the frozen max_trace_length budget while still hashing
    // at least once (so the computation always exercises the code under test).
    // This clamping is pure I/O plumbing; the chain algorithm is unchanged.
    if input.len() < 33 {
        return Vec::new();
    }
    let mut msg = [0u8; 32];
    msg.copy_from_slice(&input[0..32]);
    let num_iters: u32 = (input[32] % 8) as u32 + 1;

    // (2) CALL -----------------------------------------------------------------
    // Original benchmark body, byte-for-byte:
    let mut hash = msg;
    for _ in 0..num_iters {
        hash = jolt_inlines_keccak256::Keccak256::digest(&hash);
    }

    // (3) ENCODE ---------------------------------------------------------------
    // The original return type is [u8; 32]; serialise it as its raw bytes.
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&hash);
    out
}
