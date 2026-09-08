#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    use core::hint::black_box;

    // (1) DECODE ---------------------------------------------------------------
    // Byte layout: 3 * u32 little-endian = 12 bytes total.
    //   a = input[0..4]  (u32 LE)
    //   b = input[4..8]  (u32 LE)
    //   c = input[8..12] (u32 LE)
    // Total-decode guard: short input returns an empty Vec instead of panicking.
    if input.len() < 12 {
        return Vec::new();
    }
    let a = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);
    let b = u32::from_le_bytes([input[4], input[5], input[6], input[7]]);
    let c = u32::from_le_bytes([input[8], input[9], input[10], input[11]]);

    // Guard division-by-zero (would panic identically on both arms; skip to
    // avoid wasting inputs that never reach the computation under test).
    if c == 0 {
        return Vec::new();
    }

    // (2) CALL -----------------------------------------------------------------
    // Lifted verbatim from the muldiv benchmark's provable body.
    let result: u32 = black_box(a * b / c);

    // (3) ENCODE ---------------------------------------------------------------
    let mut out = Vec::with_capacity(4);
    out.extend_from_slice(&result.to_le_bytes());
    out
}
