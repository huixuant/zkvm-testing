#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // Byte layout: bytes[0..4] = i32 (little-endian) = n.
    // Total 4 bytes. Short input (< 4 bytes) => return empty Vec (no panic).
    if input.len() < 4 {
        return Vec::new();
    }
    let n = i32::from_le_bytes([input[0], input[1], input[2], input[3]]);

    // (2) CALL -----------------------------------------------------------------
    // Lifted body of stdlib's `string_concat` provable function, verbatim
    // computation (the original `println!` was a host-only side effect and does
    // not influence the returned value, so it is omitted). Concatenates the
    // decimal representations of 0..n into one String.
    let mut res = String::new();
    for i in 0..n {
        res += &i.to_string();
    }

    // (3) ENCODE ---------------------------------------------------------------
    // The result String's UTF-8 bytes are the output.
    res.into_bytes()
}
