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
    // Byte layout: the ENTIRE input buffer is the message to hash. The original
    // benchmark's provable fn was `sha3(input: &[u8]) -> [u8; 32]`, so the
    // reconstructed argument is just `&input[..]` (variable length, 0..=65536
    // bytes). No fixed header; any length (including empty) is valid.
    let message: &[u8] = &input;

    // (2) CALL -----------------------------------------------------------------
    // Lifted verbatim from the sha3-ex provable body:
    //   jolt_inlines_keccak256::Keccak256::digest(input)
    let digest: [u8; 32] = jolt_inlines_keccak256::Keccak256::digest(message);

    // (3) ENCODE ---------------------------------------------------------------
    // The original returned [u8; 32]; serialise those 32 bytes in order.
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&digest);
    out
}
