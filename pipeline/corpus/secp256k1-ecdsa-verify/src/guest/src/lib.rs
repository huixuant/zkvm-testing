#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

use jolt_inlines_secp256k1::{ecdsa_verify, Secp256k1Fr, Secp256k1Point, UnwrapOrSpoilProof};

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(max_input_size = 65536, max_output_size = 65536, stack_size = 8388608)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // Byte layout — 20 little-endian u64 = 160 bytes total. Read by the
    // input-generator to encode secp256k1 ECDSA test vectors:
    //   z : [u64; 4]  bytes   0..32   (message hash, scalar field Fr)
    //   r : [u64; 4]  bytes  32..64   (signature r, scalar field Fr)
    //   s : [u64; 4]  bytes  64..96   (signature s, scalar field Fr)
    //   q : [u64; 8]  bytes  96..160  (public key point: x[u64;4] then y[u64;4])
    // Each [u64; N] is N consecutive little-endian u64 limbs (limb 0 = least
    // significant). DECODE is total: a short input yields an empty Vec instead
    // of panicking, so truncated vectors never reach the code under test.
    if input.len() < 160 {
        return Vec::new();
    }
    let mut w = [0u64; 20];
    let mut i = 0;
    while i < 20 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&input[i * 8..i * 8 + 8]);
        w[i] = u64::from_le_bytes(b);
        i += 1;
    }
    let z: [u64; 4] = [w[0], w[1], w[2], w[3]];
    let r: [u64; 4] = [w[4], w[5], w[6], w[7]];
    let s: [u64; 4] = [w[8], w[9], w[10], w[11]];
    let q: [u64; 8] = [w[12], w[13], w[14], w[15], w[16], w[17], w[18], w[19]];

    // (2) CALL -----------------------------------------------------------------
    // Lifted verbatim from the benchmark's provable body. `from_u64_arr`
    // validates the field/point encodings and spoils the proof on malformed
    // input, exactly as the original. The only deviation is the final line: the
    // benchmark did `ecdsa_verify(..).unwrap_or_spoil_proof()` (returning `()`),
    // whereas we capture the `Result` so its Ok/Err outcome is observable in the
    // output bytes. The computation itself is byte-for-byte the original.
    let z = Secp256k1Fr::from_u64_arr(&z).unwrap_or_spoil_proof();
    let r = Secp256k1Fr::from_u64_arr(&r).unwrap_or_spoil_proof();
    let s = Secp256k1Fr::from_u64_arr(&s).unwrap_or_spoil_proof();
    let q = Secp256k1Point::from_u64_arr(&q).unwrap_or_spoil_proof();
    let verified = ecdsa_verify(z, r, s, q).is_ok();

    // (3) ENCODE ---------------------------------------------------------------
    // 1 byte: 1 = Ok(()) (signature valid), 0 = Err(..) (verification failed).
    let mut out = Vec::with_capacity(1);
    out.push(u8::from(verified));
    out
}
