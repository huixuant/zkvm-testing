// ============================================================================
// Test case: issue-113407 / emission-halt-guard
// Rust issue: https://github.com/rust-lang/rust/issues/113407
//
// WHAT THIS DOES (real-world zkVM workload):
//   Block-reward minting for a proof-of-stake chain (blockchain / tokenomics).
//   The token has a decaying emission schedule: the per-block emission rate is a
//   committed protocol constant that shrinks over time and is meant to reach zero
//   once the schedule ends (a "tail emission" that tapers out). Each block the
//   reward engine narrows the rate to the `f32` it mints in; when the rate has
//   decayed to zero, emission has ended and NO new tokens are minted this block;
//   otherwise it mints the block reward to each validator. Here the committed rate
//   is past the end of the schedule — a tiny value that should narrow to zero.
//
//   This is the natural home for the bug: real emission schedules are hard-coded
//   constants that decay toward zero, so "narrow the committed rate; if it is now
//   zero, stop minting" is exactly how a reward engine is written.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   The rate is a compile-time constant `f64::from_bits(0x1987_3cc2) as f32`, so
//   the narrowing cast is folded at compile time by MIR const-propagation
//   (mir-opt-level >= 2, on by default in release). The 1.94-era const-eval path
//   uses `rustc_apfloat`, which mis-rounds this subnormal f64->f32 conversion
//   (fixed by PR #113843 / rustc_apfloat#1): it yields a non-zero subnormal `f32`
//   where the correct result flushes to `0.0`. (Load-bearing: the rate must be a
//   constant so the cast is const-folded — a runtime cast on the RISC-V FPU would
//   round correctly; the exact bit pattern and the f64->f32 narrowing are required.)
//   Correct: `rate32 == 0.0` is true  -> emission ended -> mint nothing.
//   Wrong:   `rate32` is a non-zero subnormal -> the `== 0.0` test is false ->
//   the block reward is minted.
//
// SECURITY IMPACT:
//   Emission that the committed schedule has ended is silently resumed: the chain
//   mints block rewards it should never mint. A valid proof commits an inflated
//   token supply — unauthorized minting / supply inflation past the hard cap, with
//   validators credited rewards the protocol did not authorize.
// ============================================================================
#![cfg_attr(feature = "guest", no_std)]

/// Committed per-block emission rate (bit-exact) for this block height. The
/// schedule has ended, so narrowed to f32 this should underflow to zero.
const EMISSION_RATE_BITS: u64 = 0x1987_3cc2;

#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn mint_block_rewards(validators: u32, base_reward: u32) -> u32 {
    // Effective emission rate for this block, in the f32 the mint engine computes in.
    let rate32 = f64::from_bits(EMISSION_RATE_BITS) as f32;

    if rate32 == 0.0 {
        // Emission schedule has tapered to zero: mint nothing this block.
        0
    } else {
        // Phantom emission: the block reward is minted to every validator.
        validators.wrapping_mul(base_reward)
    }
}
