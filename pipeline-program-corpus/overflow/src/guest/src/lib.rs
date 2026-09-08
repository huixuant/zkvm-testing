#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(stack_size = 8192, heap_size = 65536, max_trace_length = 65536)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // The corpus program `overflow` exposes provable functions that take NO
    // arguments; its computation is a fixed stack-array reduction. There is
    // therefore nothing to decode from `input` and no layout to honour. `input`
    // is accepted and ignored for any length (decode is total: no panic path).
    let _ = &input;

    // (2) CALL -----------------------------------------------------------------
    // Lifted verbatim from `overflow::guest::overflow_stack` (corpus src/overflow):
    //     let arr = [1u32; 1024];
    //     arr.iter().sum()
    // The computation between DECODE and ENCODE is byte-for-byte the original.
    let arr = [1u32; 1024];
    let result: u32 = arr.iter().sum();

    // (3) ENCODE ---------------------------------------------------------------
    // Original return type is u32; serialise as 4 little-endian bytes.
    let mut out = Vec::with_capacity(4);
    out.extend_from_slice(&result.to_le_bytes());
    out
}
