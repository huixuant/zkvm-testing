#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

/// Fast hash function implementation using wyhash64 algorithm.
///
/// This function provides a high-quality, fast hash suitable for hashmap operations.
/// It uses the wyhash64 algorithm which offers good distribution properties and
/// performance characteristics.
fn wyhash64(mut x: u64) -> u64 {
    x ^= x >> 32;
    x = x.wrapping_mul(0xd6e8feb86659fd93);
    x ^= x >> 32;
    x = x.wrapping_mul(0xd6e8feb86659fd93);
    x
}

/// Lifted verbatim from the `btreemap` benchmark's `#[jolt::provable]` body
/// (corpus/src/btreemap/guest/src/lib.rs). Computation is byte-for-byte the
/// original; only the surrounding I/O plumbing changed. `#[inline(never)]` keeps
/// it resolvable as a symbol for `--trace-fn` under `lto = "fat"`.
#[inline(never)]
fn btreemap(n: u32) -> u128 {
    use alloc::collections::BTreeMap;

    let mut map = BTreeMap::new();
    let mut inserted_keys = alloc::vec::Vec::with_capacity(n as usize);

    // Phase 1: Insert N entries with high-entropy keys
    for i in 0..n {
        let key = wyhash64(i as u64); // Use u64 directly, not usize
        inserted_keys.push(key);
        map.insert(key, i as u64);
    }

    // Phase 2: Delete 25% of the inserted keys to trigger rebalancing
    let delete_count = n / 4;
    for i in 0..delete_count {
        let key = inserted_keys[i as usize];
        map.remove(&key);
    }

    // Phase 3: Insert N/2 new entries with new hashed keys
    for i in 0..(n / 2) {
        let key = wyhash64((i + n * 2) as u64); // Non-overlapping seed
        map.insert(key, (i + n) as u64);
    }

    // Phase 4: Range scan over middle 25% of key space
    let mut range_sum = 0u64;
    if let Some((&min_key, _)) = map.first_key_value() {
        if let Some((&max_key, _)) = map.last_key_value() {
            let range_size = (max_key - min_key) / 4;
            let start = min_key + range_size;
            let end = start + range_size;

            for (_, value) in map.range(start..end) {
                range_sum = range_sum.wrapping_add(*value);
            }
        }
    }

    // Combine size and range sum into a single return value
    (map.len() as u128).wrapping_add(range_sum as u128)
}

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(max_input_size = 65536, max_output_size = 65536, stack_size = 8388608)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // INPUT BYTE LAYOUT (read by the input generator):
    //
    //   offset 0..4 : u32, little-endian  -> raw_n
    //
    // Total required length: 4 bytes. Shorter input => return empty Vec (total
    // decode, never panics).
    //
    // The single argument of the code under test is `n: u32`. It is derived as
    //
    //   n = raw_n % 64            (i.e. 0 <= n <= 63)
    //
    // The modulus keeps the work inside the harness's FROZEN sizing attributes
    // (heap_size = 32768, max_trace_length = 65536); the upstream benchmark's own
    // host drives this function with n = 50, so 0..=63 covers the intended
    // operating range. Every one of the 2^32 possible raw_n values decodes to a
    // valid n, so the generator may emit any 4 bytes.
    if input.len() < 4 {
        return Vec::new();
    }
    let raw_n = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);
    let n: u32 = raw_n % 64;

    // (2) CALL -----------------------------------------------------------------
    let result: u128 = btreemap(n);

    // (3) ENCODE ---------------------------------------------------------------
    // 16 bytes, u128 little-endian.
    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(&result.to_le_bytes());
    out
}
