// ============================================================================
// Test case: issue-150904 / range-credential
// Rust issue: https://github.com/rust-lang/rust/issues/150904
//
// WHAT THIS DOES (real-world zkVM workload):
//   Proof of identity / selective-disclosure credential check. A user proves,
//   without revealing the underlying values, that their committed attributes
//   satisfy an access policy: a range proof on age (age >= min_age, e.g. the
//   "over 18" gate), an income threshold (income >= min_income), and a region
//   attribute match (region == allowed_region). The provable function returns 1
//   for "credential granted / eligible" and 0 for "denied". The host proves an
//   applicant who is in the WRONG region and must be denied.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   The policy clauses are folded into one mutated boolean inside an
//   `#[inline(always)]` helper, the same non-SSA-boolean-from-integer-compares
//   shape as the reproducer:
//       let mut ok = age >= min_age;            // u64 >=, true here
//       ok &= region == allowed_region;         // u64 ==, FALSE here (wrong region)
//       ok &= income >= min_income;             // u64 >=, true here
//   After inlining, `ok` (reassigned by `&=`, so non-SSA) feeds the
//   `if ok { 1 } else { 0 }` SwitchInt. `SimplifyComparisonIntegral` simplified
//   that SwitchInt against a comparison whose place was mutated before the
//   terminator, dropping the `region == allowed_region` clause.
//     Correct result for check_eligibility(25, 18, 90000, 50000, 44, 7): 0
//       (deny; region 44 != allowed 7). Buggy 1.92+ -O result: 1 (grant).
//
// SECURITY IMPACT:
//   A valid proof of eligibility is produced for an applicant who fails the
//   policy. The verifier accepts a forged credential / membership claim,
//   granting access (or an age/region-gated right) to an ineligible party.
// ============================================================================

#![cfg_attr(feature = "guest", no_std)]

#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
pub fn check_eligibility(
    age: u64,
    region: u64,
) -> u32 {
    if policy_satisfied(age, region) {
        1
    } else {
        0
    }
}

#[inline(always)]
fn policy_satisfied(
    age: u64,
    region: u64,
) -> bool {
    let mut ok = age == 18; // range proof: age gate
    ok &= region == 7; // attribute match: region gate
    ok
}
