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
    // Byte layout (little-endian): two consecutive u64 = 16 bytes total.
    //   bytes[0..8]   = a (u64 LE)  -- the value to invert
    //   bytes[8..16]  = m (u64 LE)  -- the modulus
    // DECODE is total: short input yields an empty Vec instead of panicking.
    if input.len() < 16 {
        return Vec::new();
    }
    let mut a_bytes = [0u8; 8];
    let mut m_bytes = [0u8; 8];
    a_bytes.copy_from_slice(&input[0..8]);
    m_bytes.copy_from_slice(&input[8..16]);
    let a = u64::from_le_bytes(a_bytes);
    let m = u64::from_le_bytes(m_bytes);

    // (2) CALL -----------------------------------------------------------------
    // Lifted body of the original `#[jolt::provable] fn modinv(a, m) -> u64`.
    // The Jolt advice tape is proving-time I/O plumbing; the value it carries is
    // exactly `modinv_naive(a, m)` (guaranteed by the original's
    // `assert_eq!(inv_advice, inv_naive)`). So we compute the advice value
    // directly (this is the body of the original `#[jolt::advice] modinv_advice`)
    // and replace `check_advice!` with an `assert!` that performs the identical
    // verification, preserving the original panic-on-failure semantics
    // byte-for-byte. No dependency crates are referenced by the lifted logic.
    let result: u64 = modinv(a, m);

    // (3) ENCODE ---------------------------------------------------------------
    // Original return type is u64 -> 8 bytes little-endian.
    let mut out = Vec::with_capacity(8);
    out.extend_from_slice(&result.to_le_bytes());
    out
}

fn modinv(a: u64, m: u64) -> u64 {
    // Advice value computed directly (original `modinv_advice` body).
    let inv = modinv_naive(a, m);
    let quo = if m == 0 {
        0
    } else {
        (a as u128 * inv as u128 / m as u128) as u64
    };

    let inv_advice = {
        // Verify the advice is correct (original `check_advice!`): a * inv ≡ 1
        // (mod m) and inv < m. Panics on failure, matching the original.
        let product = (a as u128) * (inv as u128) - (quo as u128) * (m as u128);
        assert!(product == 1u128 && inv < m);
        inv
    };

    let inv_naive = modinv_naive(a, m);

    assert_eq!(inv_advice, inv_naive);

    inv_advice
}

fn modinv_naive(a: u64, m: u64) -> u64 {
    if m == 0 {
        return 0;
    }

    // Extended Euclidean Algorithm
    let (mut old_r, mut r) = (a as i128, m as i128);
    let (mut old_s, mut s) = (1i128, 0i128);

    while r != 0 {
        let quotient = old_r / r;
        (old_r, r) = (r, old_r - quotient * r);
        (old_s, s) = (s, old_s - quotient * s);
    }

    // old_r is the GCD
    if old_r != 1 {
        // No inverse exists
        return 0;
    }

    // Ensure the result is positive
    if old_s < 0 {
        (old_s + m as i128) as u64
    } else {
        old_s as u64
    }
}
