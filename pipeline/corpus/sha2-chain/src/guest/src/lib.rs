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
    // Original benchmark signature: sha2_chain(input: [u8; 32], num_iters: u32).
    // Byte layout (little-endian, total 36 bytes):
    //   [0..32]  : input   — the 32-byte seed hash ([u8; 32])
    //   [32..36] : num_iters — u32 LE, number of SHA-256 iterations
    // DECODE is total: on short input return an empty Vec instead of panicking.
    if input.len() < 36 {
        return Vec::new();
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&input[0..32]);
    let num_iters = u32::from_le_bytes([input[32], input[33], input[34], input[35]]);

    // (2) CALL -----------------------------------------------------------------
    // Byte-for-byte the original body of sha2_chain.
    let mut hash = seed;
    for _ in 0..num_iters {
        hash = jolt_inlines_sha2::Sha256::digest(&hash);
    }

    // (3) ENCODE ---------------------------------------------------------------
    // Original return value is [u8; 32]; serialise it as its raw bytes.
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&hash);
    out
}
