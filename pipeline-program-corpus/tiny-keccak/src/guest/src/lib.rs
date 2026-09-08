#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
// #[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
#[jolt::provable(max_input_size = 65536, max_output_size = 65536, stack_size = 8388608)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // Byte layout: the entire `input` buffer IS the message to be hashed
    // (variable length, 0..=65536 bytes). No header, no length prefix — every
    // byte of `input` is fed verbatim to the hasher. Decode is total: an empty
    // input hashes the empty message (well-defined), never panics.
    let message: &[u8] = &input;

    // (2) CALL -----------------------------------------------------------------
    // Compute SHA3-256 over the message using tiny-keccak's public API.
    use tiny_keccak::{Hasher, Sha3};
    let mut hasher = Sha3::v256();
    hasher.update(message);
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);

    // (3) ENCODE ---------------------------------------------------------------
    // Output = the 32-byte SHA3-256 digest.
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&digest);
    out
}
