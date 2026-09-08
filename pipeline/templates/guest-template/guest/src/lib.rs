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
    // Turn `input` bytes into the arguments the code under test expects.
    // (per-crate step fills this in)

    // (2) CALL -----------------------------------------------------------------
    // Invoke the code under test.
    //
    // NOTE on --trace-fn: reachability is resolved from the guest ELF symbol
    // table, so it can only see functions that still EXIST as symbols. Under
    // `lto = "fat"` + `-Copt-level=3` a small callee is usually inlined away and
    // will report `reached=unknown`. If you need a definite true/false for a
    // specific function, give that function `#[inline(never)]` in the crate under
    // test (or wrap the call in a local `#[inline(never)]` shim) so it survives
    // as its own symbol.
    // Placeholder: deterministic and toolchain-independent so the two compiler
    // arms MUST agree here (null control). Replaced per crate.
    let result: u64 = input.len() as u64;

    // (3) ENCODE ---------------------------------------------------------------
    // Turn the result back into output bytes.
    let mut out = Vec::with_capacity(8);
    out.extend_from_slice(&result.to_le_bytes());
    out
}
