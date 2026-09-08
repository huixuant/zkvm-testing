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
    // Byte layout: the entire `input` byte string IS the SHA-256 message.
    //   input: Vec<u8>  ==  message: &[u8]  (variable length, 0..=N bytes)
    // No fixed-size fields, so no short-input guard is needed: every possible
    // byte string (including empty) is a valid message, so DECODE is total.
    let message: &[u8] = input.as_slice();

    // (2) CALL -----------------------------------------------------------------
    // Lifted verbatim from the sha2-ex benchmark's provable body:
    //   fn sha2(input: &[u8]) -> [u8; 32] { jolt_inlines_sha2::Sha256::digest(input) }
    let digest: [u8; 32] = jolt_inlines_sha2::Sha256::digest(message);

    // (3) ENCODE ---------------------------------------------------------------
    // Turn the 32-byte digest back into output bytes.
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&digest);
    out
}
