#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

use crypto_bigint::subtle::ConstantTimeGreater;
use crypto_bigint::U256;

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // Byte layout (little-endian), total 64 bytes:
    //   offset  0..32 : a : U256, 32 bytes little-endian
    //   offset 32..64 : b : U256, 32 bytes little-endian
    // Short input (< 64 bytes) yields an empty output instead of panicking, so
    // the code under test is never reached with a truncated argument.
    if input.len() < 64 {
        return Vec::new();
    }
    let a = U256::from_le_slice(&input[0..32]);
    let b = U256::from_le_slice(&input[32..64]);

    // (2) CALL -----------------------------------------------------------------
    // Truncated 256-bit product plus a constant-time comparison. The comparison
    // exercises crypto-bigint's `subtle::Choice` bitwise-accumulation paths.
    let product = a.wrapping_mul(&b);
    let a_gt_b: u8 = a.ct_gt(&b).unwrap_u8();

    // (3) ENCODE ---------------------------------------------------------------
    // Output layout (33 bytes): product's 4 words (LE, 32 bytes) then the
    // comparison result byte (1 = a > b, 0 = otherwise).
    let mut out = Vec::with_capacity(33);
    for word in product.to_words() {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.push(a_gt_b);
    out
}
