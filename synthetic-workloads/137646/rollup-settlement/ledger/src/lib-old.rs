#![no_std]
#![allow(dead_code)]

// ============================================================================
// Test case: issue-137646 / rollup-settlement  (triggering-core library crate)
// Rust issue: https://github.com/rust-lang/rust/issues/137646
//
// This library crate holds the LOAD-BEARING miscompilation core for issue
// #137646; the guest crate (../guest) routes a realistic rollup settlement
// workload through `settled_amount()`. Keeping the core in a *separate library
// crate* is one of the required trigger conditions (the bug needs the trait/impl
// split across crates, opt-level>=1, incremental=false — see guest/Cargo.toml).
//
// The triggering SHAPE is carried over verbatim from the reproducer
// (~/msc-ip/jolt/test-137646/test/src/lib.rs):
//   * a 3×i32 struct passed BY VALUE,
//   * to a VIRTUAL trait-object method (`&dyn Settler`),
//   * whose default body takes a SHARED borrow `&tx.0` (load-bearing: changing
//     it to `&mut tx.0` is the documented "fix"),
//   * a concrete `impl Settler for ()` that WRITES the by-value arg,
//   * and an opaque (`format!`-guarded) read that defeats const-folding so the
//     leaked value is actually observed.
//
// WHAT CHANGED VS. THE RAW REPRODUCER (realism, not mechanism):
//   The reproducer writes `tx.0 = 42` — a conspicuous magic-number TEST MARKER
//   whose only purpose is to make the leak unmistakable. That is exactly the
//   kind of line a source audit would flag, so it is a poor model of how this
//   bug bites in practice. Here the write is instead a perfectly ordinary
//   operation: the settlement rule normalizes the human-entered credit into
//   protocol BASE UNITS (fixed-point scaling) on its OWN owned copy of the
//   record, in order to range-check it against the per-batch cap. Mutating a
//   by-value parameter you own is idiomatic Rust, and the caller's committed
//   credit is *supposed* to be unaffected — which is precisely the (correct)
//   assumption the miscompiler violates. No malicious or suspicious code is
//   required; correct, review-clean source is silently corrupted.
//
// On the pinned 1.94 toolchain the vtable call's argument is wrongly deduced
// `readonly`, so the caller reuses the stack slot of the identical `Tx(1,2,3)`
// literal and the post-call read returns the NORMALIZED amount instead of the
// original. Correct: committed credit == 1. Buggy: credit == 1 * BASE_UNIT_SCALE.
// ============================================================================

extern crate alloc;
use alloc::format;

/// Protocol fixed-point scale: amounts are quoted to 6 decimal places, so the
/// canonical on-ledger representation of a credit is `human_amount * 1_000_000`.
const BASE_UNIT_SCALE: i32 = 1_000_000;

/// Per-batch credit cap, expressed in base units. Credits above this are
/// rejected by the settlement rule.
const PER_BATCH_CAP: i32 = 5_000_000;

/// Settlement record: (credit_amount, from_account, to_account).
pub struct Tx(i32, i32, i32);

/// A pluggable settlement / fraud-check rule, selected at runtime via dynamic
/// dispatch. The default rule only *inspects* the record (read-only).
pub trait Settler {
    fn validate(&self, mut tx: Tx) {
        // Default rule: inspect only. Read the credit through a SHARED borrow to
        // sanity-check its sign. LOAD-BEARING: this must stay a shared borrow of
        // `tx.0`. Promoting it to `&mut tx.0` is exactly the upstream fix and
        // removes the bug.
        let _credit = &tx.0;
        debug_assert!(*_credit >= 0);
    }
}

/// The production settlement rule actually installed for this batch.
impl Settler for () {
    fn validate(&self, mut tx: Tx) {
        // Normalize the human-entered credit into protocol base units so it can
        // be range-checked against the per-batch cap. This operates on THIS
        // rule's owned copy of the record — by design the caller's committed
        // credit is left untouched (on a correct compiler).
        tx.0 = tx.0.wrapping_mul(BASE_UNIT_SCALE);
        debug_assert!(tx.0 <= PER_BATCH_CAP);
    }
}

/// Read the credit amount back out. `format!` forces a genuine runtime read of
/// `tx.0` (prevents the constant `Tx(1,2,3)` from being folded to 1).
fn read_amount(tx: Tx) -> i32 {
    if format!("{}", tx.0) != "1" {
        tx.0
    } else {
        1
    }
}

/// Run the settlement step: inspect the record, hand it to the dispatched rule,
/// then read the (supposedly unchanged) committed credit back out.
pub fn settle(settler: &dyn Settler) -> i32 {
    let _ = read_amount(Tx(1, 2, 3));
    settler.validate(Tx(1, 2, 3));
    read_amount(Tx(1, 2, 3))
}

/// Committed credit for this batch == 1 on a correct compiler; the normalized
/// `1 * BASE_UNIT_SCALE` (== 1_000_000) once #137646 leaks the rule's by-value
/// write back into the caller's slot.
pub fn settled_amount() -> i32 {
    settle(&())
}
