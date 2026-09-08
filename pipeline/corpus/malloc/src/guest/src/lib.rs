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
    // Byte layout: 1 * i32 little-endian (4 bytes) = `val`, the sole argument of
    // the original benchmark's provable fn `alloc(val: i32) -> u32`.
    // DECODE is total: short input yields an empty Vec instead of panicking.
    if input.len() < 4 {
        return Vec::new();
    }
    let val = i32::from_le_bytes([input[0], input[1], input[2], input[3]]);

    // (2) CALL -----------------------------------------------------------------
    // Lifted verbatim from the malloc benchmark's `#[jolt::provable] fn alloc`.
    // The C functions are linked by the build script (guest/build.rs compiling
    // guest/src/malloc.c); malloc/free are provided by the Jolt runtime via the
    // `malloc-shim` feature on the jolt-sdk dependency.
    extern "C" {
        fn alloc_and_set(val: i32) -> *mut i32;
        fn free_me(ptr: *mut i32);
    }

    let ptr = unsafe { alloc_and_set(val) };

    // The value should have been set correctly
    assert_eq!(unsafe { *ptr }, val);

    unsafe { free_me(ptr) };

    let result: u32 = 0;

    // (3) ENCODE ---------------------------------------------------------------
    // u32 little-endian (4 bytes).
    let mut out = Vec::with_capacity(4);
    out.extend_from_slice(&result.to_le_bytes());
    out
}
