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
    // Program under test: corpus `backtrace` (guest-nostd), whose original
    // provable signature is `panic_backtrace_nostd(should_panic: bool) -> u32`.
    //
    // BYTE LAYOUT (little-endian, total = 1 byte):
    //   offset 0, len 1 : u8 flag -> `should_panic: bool`
    //                     canonical bool encoding: 1 => true, anything else
    //                     (including 0) => false.
    // Trailing bytes beyond offset 0 are ignored.
    // Short input (len < 1) returns an empty Vec instead of panicking, so a
    // truncated vector never reaches the code under test.
    if input.is_empty() {
        return Vec::new();
    }
    let should_panic: bool = input[0] == 1;

    // (2) CALL -----------------------------------------------------------------
    // Body lifted verbatim from `panic_backtrace_nostd` in
    // corpus/src/backtrace/guest-nostd/src/lib.rs (computation unchanged).
    let result: u32 = panic_backtrace_nostd(should_panic);

    // (3) ENCODE ---------------------------------------------------------------
    // u32 result, little-endian, 4 bytes.
    let mut out = Vec::with_capacity(4);
    out.extend_from_slice(&result.to_le_bytes());
    out
}

/// no-std backtrace demo: intentionally panic after a few stack frames.
/// Note: Functions use stack arrays to prevent tail-call optimization,
/// ensuring proper frame pointers are generated for unwinding.
#[inline(never)]
fn panic_backtrace_nostd(should_panic: bool) -> u32 {
    if should_panic {
        level_one();
    }
    7
}

#[inline(never)]
fn level_one() {
    // Stack allocation prevents tail-call optimization
    let arr = [0u8; 32];
    core::hint::black_box(&arr);
    level_two();
}

#[inline(never)]
fn level_two() {
    // Stack allocation prevents tail-call optimization
    let arr = [0u8; 32];
    core::hint::black_box(&arr);
    panic!("backtrace demo (no-std)");
}
