// ============================================================================
// Test case: issue-150904 / sig-precheck
// Rust issue: https://github.com/rust-lang/rust/issues/150904
//
// WHAT THIS DOES (real-world zkVM workload):
//   ECDSA-style transaction signature verifier for a zkVM-native rollup. The
//   guest hashes the transaction fields (chain_id, nonce, value, gas,
//   data_hash) using a sequence of multiply-rotate-XOR rounds (a lightweight
//   hash for RISC-V), checks that the result matches the committed message hash,
//   then validates the signature scalar components (format version, r non-zero,
//   s non-zero, both within the secp256k1 group order). If all checks pass it
//   returns a "sigcheck commitment" — H(r || s || msg_hash) — that downstream
//   logic can use to enforce binding between the proven hash and the signature.
//   The host proves a transaction signed with a degenerate signature (s == 0)
//   that MUST be rejected.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   The signature-component checks are folded into one mutated boolean inside an
//   `#[inline(always)]` helper, matching the upstream reproducer:
//       let mut ok = sig_format == SIG_FMT_SECP256K1;  // u64 == CONSTANT (1)
//       ok &= r != 0;                                    // u64 !=, true (r = 5)
//       ok &= s != 0;                                    // u64 !=, FALSE (s == 0)
//       ok &= r < SECP256K1_ORDER_HI;                   // u64 <, true
//       ok &= s < SECP256K1_ORDER_HI;                   // u64 <, true (s = 0 < order)
//   The first comparison uses `==` against compile-time constant SIG_FMT_SECP256K1.
//   `SimplifyComparisonIntegral` traces the SwitchInt back to `sig_format == 1`
//   and drops all subsequent `ok &= ...` mutations — silently eliminating the
//   r/s validity checks.
//     Correct result: 0 (reject; s == 0 is a degenerate/forged scalar).
//     Buggy 1.92+ -O result: non-zero commitment (degenerate sig accepted).
//
// SECURITY IMPACT:
//   A valid proof is produced that certifies a degenerate (s == 0) signature as
//   structurally valid. The rollup verifier accepts the sigcheck commitment and
//   routes the transaction to downstream curve arithmetic; a carefully crafted
//   degenerate signature can then forge arbitrary transactions without knowing
//   the private key, breaking authentication for every account on the rollup.
// ============================================================================

#![cfg_attr(feature = "guest", no_std)]

const SIG_FMT_SECP256K1: u64 = 1;
const SECP256K1_ORDER_HI: u64 = 0xFFFF_FFFE_FFFF_FFFF;

#[jolt::provable(heap_size = 65536, max_trace_length = 131072)]
pub fn verify_tx_signature(
    sig_format: u64,
    r: u64,
    s: u64,
    tx_chain_id: u64,
    tx_nonce: u64,
    tx_value: u64,
    tx_gas: u64,
    tx_data_hash: u64,
    expected_msg_hash: u64,
) -> u64 {
    // Compute message hash: multiply-rotate-XOR over transaction fields
    let msg_hash = compute_msg_hash(tx_chain_id, tx_nonce, tx_value, tx_gas, tx_data_hash);

    // Reject if the prover supplied an inconsistent message hash commitment
    if msg_hash != expected_msg_hash {
        return 0;
    }

    if !sig_components_valid(sig_format, r, s) {
        return 0;
    }

    // Return sigcheck commitment: H(r || s || msg_hash)
    r.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ s.wrapping_mul(0x6C62_272E_07BB_0142)
        ^ msg_hash.wrapping_mul(0xBF58_476D_1CE4_E5B9)
}

#[inline(always)]
fn sig_components_valid(sig_format: u64, r: u64, s: u64) -> bool {
    let mut ok = sig_format == 1; // == CONSTANT: load-bearing trigger
    ok &= r != 0;                                   // r must be non-zero
    ok &= s != 0;                                   // s must be non-zero (VIOLATED: s == 0)
    ok &= r < SECP256K1_ORDER_HI;                  // r within group order
    ok &= s < SECP256K1_ORDER_HI;                  // s within group order
    ok
}

fn compute_msg_hash(chain_id: u64, nonce: u64, value: u64, gas: u64, data_hash: u64) -> u64 {
    let mut h: u64 = 0x0123_4567_89AB_CDEF;
    h ^= chain_id.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = h.rotate_left(17) ^ nonce.wrapping_mul(0x6C62_272E_07BB_0142);
    h = h.rotate_left(13) ^ value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = h.rotate_left(19) ^ gas.wrapping_mul(0x94D0_49BB_1331_11EB);
    h = h.rotate_left(7) ^ data_hash.wrapping_mul(0xDEAD_BEEF_CAFE_1234);
    h ^= h >> 32;
    h
}
