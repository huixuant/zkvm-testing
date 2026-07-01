// ============================================================================
// Test case: issue-150904 / range-credential
// Rust issue: https://github.com/rust-lang/rust/issues/150904
//
// WHAT THIS DOES (real-world zkVM workload):
//   Zero-knowledge KYC credential verifier for a DeFi lending protocol. A
//   borrower proves inside the guest that their off-chain KYC credential
//   satisfies the protocol's lending policy: it is a V1 KYC credential (type
//   constant), the applicant is 18 or older, the requested loan-to-value ratio
//   (LTV, computed from collateral and loan amount) stays within the protocol
//   cap, and their income exceeds the minimum floor. If all checks pass the
//   function returns a unique per-user nullifier (preventing double-borrowing);
//   otherwise it returns 0. The host proves a borrower whose LTV (80 000 bps =
//   800%) far exceeds the 75% cap and MUST be rejected.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   The lending policy clauses are folded into one mutated boolean inside an
//   `#[inline(always)]` helper, matching the upstream reproducer:
//       let mut ok = cred_type == KYC_TYPE_V1;  // u64 == CONSTANT (1)
//       ok &= age >= min_age;                    // u64 >=, true (25 >= 18)
//       ok &= ltv_bps <= max_ltv_bps;           // u64 <=, FALSE (80_000 > 7_500)
//       ok &= income >= min_income;              // u64 >=, true
//   The first comparison uses `==` against compile-time constant KYC_TYPE_V1.
//   `SimplifyComparisonIntegral` traces the SwitchInt back to `cred_type == 1`
//   and drops the subsequent `ok &= ...` mutations — silently eliminating the
//   LTV cap and income floor checks.
//     Correct result: 0 (reject; LTV 800% > 75% cap).
//     Buggy 1.92+ -O result: non-zero nullifier (borrower accepted).
//
// SECURITY IMPACT:
//   A valid proof is produced for a borrower who violates the LTV cap. The
//   lending protocol accepts the loan, issuing undercollateralised credit.
//   The verifier cannot detect the policy bypass from the proof alone.
// ============================================================================

#![cfg_attr(feature = "guest", no_std)]

// const KYC_TYPE_V1: u64 = 1;

#[jolt::provable(heap_size = 65536, max_trace_length = 131072)]
pub fn verify_lending_credential(
    cred_type: u64,
    age: u64,
    min_age: u64,
    collateral: u64,
    loan_amount: u64,
    max_ltv_bps: u64,
    income: u64,
    min_income: u64,
    user_id: u64,
    protocol_salt: u64,
) -> u64 {
    // Compute LTV in basis points (loan / collateral * 10_000)
    let ltv_bps = if collateral == 0 {
        u64::MAX
    } else {
        loan_amount.saturating_mul(10_000) / collateral
    };

    // Derive nullifier: H(user_id || protocol_salt || cred_type)
    let nullifier = user_id
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ protocol_salt.wrapping_mul(0x6C62_272E_07BB_0142)
        ^ cred_type.wrapping_mul(0xBF58_476D_1CE4_E5B9);

    if policy_satisfied(cred_type, age, min_age, ltv_bps, max_ltv_bps, income, min_income) {
        nullifier
    } else {
        0
    }
}

#[inline(always)]
fn policy_satisfied(
    cred_type: u64,
    age: u64,
    min_age: u64,
    ltv_bps: u64,
    max_ltv_bps: u64,
    income: u64,
    min_income: u64,
) -> bool {
    let mut ok = cred_type == 1; // == CONSTANT: load-bearing trigger
    ok &= age >= min_age;                   // age gate: 18+
    ok &= ltv_bps <= max_ltv_bps;          // LTV cap (VIOLATED: 80_000 > 7_500)
    ok &= income >= min_income;             // income floor
    ok
}
