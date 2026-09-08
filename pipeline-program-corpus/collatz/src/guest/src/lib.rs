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
    // Byte layout (little-endian), 5 bytes total:
    //   [0..4]  start : u32 LE  -> widened to u128, forced >= 1
    //   [4]     count : u8      -> widened to u128
    // Derived: end = start + count, and the call is
    //   collatz_convergence_range(start, end).
    // Rationale for the width choice: the lifted algorithm is unchanged, but the
    // decode keeps `start` in u32 range and `count` in u8 range so that (a) no
    // `n == 0` ever enters the Collatz loop (which would never reach 1), and
    // (b) work stays bounded for the guest trace. Short input -> empty output.
    if input.len() < 5 {
        return Vec::new();
    }
    let start = u32::from_le_bytes([input[0], input[1], input[2], input[3]]) as u128;
    let start = if start == 0 { 1 } else { start };
    let count = input[4] as u128;
    let end = start + count;

    // (2) CALL -----------------------------------------------------------------
    let result: u128 = collatz_convergence_range(start, end);

    // (3) ENCODE ---------------------------------------------------------------
    // u128 result serialized little-endian (16 bytes).
    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(&result.to_le_bytes());
    out
}

// Lifted verbatim from the benchmark's provable body (computation unchanged).
#[inline(never)]
fn collatz_convergence_range(start: u128, end: u128) -> u128 {
    let mut max_num_steps = 0;
    for n in start..end {
        let num_steps = collatz_convergence(n);
        if num_steps > max_num_steps {
            max_num_steps = num_steps;
        }
    }
    max_num_steps
}

// Lifted verbatim from the benchmark's provable body (computation unchanged).
#[inline(never)]
fn collatz_convergence(n: u128) -> u128 {
    let mut n = n;
    let mut num_steps = 0;
    while n != 1 {
        if n.is_multiple_of(2) {
            n /= 2;
        } else {
            n += (n << 1) + 1;
        }
        num_steps += 1;
    }
    return num_steps;
}
