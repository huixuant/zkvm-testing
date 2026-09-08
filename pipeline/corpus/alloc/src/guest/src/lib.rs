#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

/// Lifted verbatim from the `alloc` benchmark's own provable function
/// (`corpus/src/alloc/guest/src/lib.rs`):
///
/// ```ignore
/// #[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
/// fn alloc(n: u32) -> u32 {
///     let mut v = Vec::<u32>::new();
///     for i in 0..100 {
///         v.push(i);
///     }
///
///     v[n as usize]
/// }
/// ```
///
/// The body below is byte-for-byte the original computation. `#[inline(never)]`
/// keeps it resolvable as its own ELF symbol for `--trace-fn`; it is applied
/// identically on both differential arms, so it cannot make them disagree.
///
/// NOTE: out-of-range `n` (>= 100) panics via the original slice index, exactly
/// as the benchmark does. That is the program's own semantics, preserved here;
/// only SHORT INPUT is guarded (in the harness DECODE step), never the index.
#[inline(never)]
fn alloc(n: u32) -> u32 {
    let mut v = Vec::<u32>::new();
    for i in 0..100 {
        v.push(i);
    }

    v[n as usize]
}

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // Original argument list: (n: u32).
    //
    // BYTE LAYOUT (total 4 bytes, little-endian, no padding):
    //   offset 0..4   n : u32  little-endian
    // Trailing bytes beyond offset 4 are ignored.
    // Total decode: fewer than 4 bytes available -> return an empty Vec rather
    // than panicking, so a short input never reaches the code under test.
    if input.len() < 4 {
        return Vec::new();
    }
    let n = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);

    // (2) CALL -----------------------------------------------------------------
    let result: u32 = alloc(n);

    // (3) ENCODE ---------------------------------------------------------------
    // u32 result, little-endian, 4 bytes.
    let mut out = Vec::with_capacity(4);
    out.extend_from_slice(&result.to_le_bytes());
    out
}
