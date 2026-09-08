#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

// Lifted verbatim from the `random` benchmark (corpus/src/random). Each uses a
// `StdRng` seeded from "entropy" / the "OS": under the Jolt guest, getrandom is
// served by jolt-platform's deterministic custom backend (fixed SEED), so both
// calls are fully determined by (a, b) and identical across the two compiler
// arms. Kept byte-for-byte; only I/O plumbing is added around them.
fn rand_v08(a: u32, b: u32) -> u32 {
    use rand_v08::Rng;
    use rand_v08::SeedableRng;
    // std_rng feature is needed for StdRng
    // getrandom feature is needed for from_entropy
    let mut rng = rand_v08::rngs::StdRng::from_entropy();
    rng.gen_range(a..b)
}

fn rand_v09(a: u32, b: u32) -> u32 {
    use rand_v09::Rng;
    use rand_v09::SeedableRng;
    // std_rng feature is needed for StdRng
    // os_rng feature is needed for from_os_rng
    let mut rng = rand_v09::rngs::StdRng::from_os_rng();
    rng.random_range(a..b)
}

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test.
///
/// Mode B (benchmark lift). The original provable function was
/// `rand(a: u32, b: u32) -> u32` whose body is `rand_v08(a, b) + rand_v09(a, b)`
/// (with cycle-tracking markers). We reconstruct its two `u32` arguments from
/// `input` and serialise its `u32` return value.
///
/// DECODE byte layout (little-endian, total / panic-free):
///   input[0..4]  -> a: u32 LE   (range lower bound)
///   input[4..8]  -> b: u32 LE   (range upper bound, exclusive)
/// If `input.len() < 8`, return an empty Vec (short input never reaches the code
/// under test). `gen_range(a..b)` requires a non-empty range, so when `b <= a`
/// we bump `b` to `a + 1` to keep the range valid; if that still cannot form a
/// range (`a == u32::MAX`), return an empty Vec. The computation between DECODE
/// and ENCODE is byte-for-byte the original benchmark body.
///
/// ENCODE: the original `u32` result as 4 little-endian bytes.
#[jolt::provable(heap_size = 32768, max_trace_length = 65536, stack_size = 8388608)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    if input.len() < 8 {
        return Vec::new();
    }
    let a = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);
    let b0 = u32::from_le_bytes([input[4], input[5], input[6], input[7]]);
    // Ensure a non-empty range for gen_range/random_range without changing the
    // computation (it is still `rand_v08(a, b) + rand_v09(a, b)`).
    let b = if b0 > a { b0 } else { a.wrapping_add(1) };
    if b <= a {
        return Vec::new();
    }

    // (2) CALL -----------------------------------------------------------------
    // Original `rand` provable-fn body, lifted verbatim.
    let result: u32 = rand_v08(a, b) + rand_v09(a, b);

    // (3) ENCODE ---------------------------------------------------------------
    let mut out = Vec::with_capacity(4);
    out.extend_from_slice(&result.to_le_bytes());
    out
}
