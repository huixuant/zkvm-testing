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
// TRIGGERING SHAPE (steffahn's upstream structuring of the same bug, which is
// more natural than the Jolt minimal reproducer):
//   * a small `#[derive(Copy)]` record passed BY VALUE,
//   * a SINGLE record value, copied by value into several calls,
//   * to a VIRTUAL trait-object method (`&dyn Settler`),
//   * whose default body only READS the argument (no write — load-bearing: it is
//     what makes the argument get deduced `readonly`),
//   * a concrete `impl Settler for ()` that WRITES its by-value copy,
//   * and FORMATTED reads (`jolt::println!`) of the SAME value before and after
//     the virtual call. The formatting macros capture the field BY REFERENCE
//     (`&tx.credit`) and pass it to an opaque, non-inlined formatter, which
//     forces the field's address to escape — so `tx` lives in addressable
//     memory and the post-call read is a genuine RELOAD of the corrupted slot.
//     (A by-value `core::hint::black_box(tx.credit)` does NOT do this: it
//     launders an already-loaded, pre-call value and the leak is not observed.)
//   The `readonly` mis-deduction lets the optimizer ELIDE the copy into the
//   virtual call (pass a pointer to the caller's record instead of a fresh
//   copy); the callee then writes through it, corrupting the caller's value,
//   so the post-call read observes the write. This is value-independent — it
//   does not rely on a constant — so the credit can come straight from input.
//
// WHAT IS DRESSED FOR REALISM (not mechanism):
//   The raw reproducer writes `struct_a.a = i32::MAX` — a conspicuous magic
//   marker an audit would flag. Here the write is an ordinary operation: the
//   settlement rule normalizes the human-entered credit into protocol BASE
//   UNITS (fixed-point scaling) on its OWN by-value copy, to range-check it
//   against the per-batch cap. Mutating a by-value parameter you own is
//   idiomatic Rust, and the caller's committed credit is *supposed* to be
//   unaffected — exactly the (correct) assumption the miscompiler violates. No
//   suspicious code is required; review-clean source is silently corrupted.
//
// On the pinned 1.94 toolchain the rule's normalization leaks back into the
// caller's record, so the committed credit is the NORMALIZED value.
// Correct: committed credit == declared credit. Buggy: credit == declared *
// BASE_UNIT_SCALE.
// ============================================================================

use jolt::println;

/// Protocol fixed-point scale: amounts are quoted to 6 decimal places, so the
/// canonical on-ledger representation of a credit is `human_amount * 1_000_000`.
const BASE_UNIT_SCALE: i32 = 1_000_000;

/// Per-batch credit cap, expressed in base units. Credits above this are
/// rejected by the settlement rule.
const PER_BATCH_CAP: i32 = 5_000_000;

/// Settlement record. `Copy` so a single canonical value can be handed by value
/// to each pipeline stage (audit, validation, commit) without re-deriving it —
/// the natural shape for a small fixed-size transaction descriptor.
#[derive(Clone, Copy)]
pub struct Tx {
    /// Credit applied to the destination account.
    pub credit: i32,
    /// Source account index.
    pub from: i32,
    /// Destination account index.
    pub to: i32,
}

/// A pluggable settlement / fraud-check rule, selected at runtime via dynamic
/// dispatch. The default rule only *inspects* the record (read-only).
pub trait Settler {
    fn validate(&self, tx: Tx) {
        // Default rule: inspect only. Reading the credit through a SHARED borrow
        // (never writing it) is LOAD-BEARING — it is what leaves the by-value
        // argument deduced `readonly`. Introducing any write here (the upstream
        // "fix") removes the bug.
        let _credit = &tx.credit;
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
        tx.credit = tx.credit.wrapping_mul(BASE_UNIT_SCALE);
        debug_assert!(tx.credit <= PER_BATCH_CAP);
    }
}

/// Write the incoming transaction to the pre-settlement audit log (a
/// side-effecting inspection over *all three* fields; neither alters nor returns
/// the record). Logging via `println!` captures the fields BY REFERENCE, which
/// forces `tx` into addressable memory — the precondition for the post-call
/// reload in `commit_credit` to observe a corrupted slot. Reads the full record,
/// unlike `commit_credit`.
fn audit_record(tx: Tx) {
    println!("audit: credit={} from={} to={}", tx.credit, tx.from, tx.to);
}

/// Commit the single credit field to the new state. Distinct from
/// `audit_record`: it touches only `tx.credit`. The `println!` captures
/// `&tx.credit` and passes it to the opaque formatter, forcing a genuine RELOAD
/// of the (possibly corrupted) credit from memory; the reloaded value is then
/// returned. A by-value `black_box(tx.credit)` does not force this reload and
/// leaves the leak unobserved.
fn commit_credit(tx: Tx) -> i32 {
    println!("commit: credit={}", tx.credit);
    tx.credit
}

/// Run one settlement step, in the usual ledger order:
///   1. record the incoming transaction in the audit trail,
///   2. apply the dispatched settlement / fraud-check rule (it inspects — and
///      may normalize — its own by-value copy of the record),
///   3. commit the canonical credit to the new state.
///
/// One canonical `tx` is copied by value into each stage. On a correct compiler
/// the credit committed in step 3 equals the declared credit, untouched by step
/// 2's by-value normalization. Under #137646 step 2's write leaks back into the
/// caller's record, so the committed credit is the forged `declared * SCALE`.
pub fn settle(settler: &dyn Settler, tx: Tx) -> i32 {
    audit_record(tx);          // 1. log the incoming record (reads all fields)
    settler.validate(tx);      // 2. dispatched fraud / settlement rule
    commit_credit(tx)          // 3. commit the canonical credit
}

/// Committed credit for `tx` == its declared credit on a correct compiler; the
/// normalized `declared * BASE_UNIT_SCALE` once #137646 leaks the rule's
/// by-value write back into the caller's record.
pub fn settled_amount(tx: Tx) -> i32 {
    settle(&(), tx)
}
