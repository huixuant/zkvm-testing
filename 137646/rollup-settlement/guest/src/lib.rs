#![cfg_attr(feature = "guest", no_std)]

// ============================================================================
// Test case: issue-137646 / rollup-settlement
// Rust issue: https://github.com/rust-lang/rust/issues/137646
//
// WHAT THIS DOES (real-world zkVM workload):
//   Rollup settlement step. A sequencer proves a per-account balance update for
//   a transaction batch. A single `Tx { credit, from, to }` record is handed to
//   a settlement/fraud-check rule selected via dynamic dispatch (`&dyn Settler`)
//   — the standard "pluggable validation rule" shape of a tx-processing
//   pipeline. The proven output is the account's new balance.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   #137646: a by-value struct argument passed to a *virtual* trait-object
//   method is wrongly deduced `readonly` (opt-level>=1, incremental=false,
//   trait/impl in a separate library crate — see ../ledger). The installed
//   `impl Settler for ()` performs a perfectly ordinary mutation of its OWNED
//   by-value copy — it normalizes the credit into protocol base units
//   (`tx.credit *= 1_000_000`) to range-check it against the per-batch cap. Per
//   the language, the caller's record is untouched; but because the call's
//   argument is deduced `readonly`, the optimizer ELIDES the copy and passes a
//   pointer to the caller's own `Tx`, so the rule's write corrupts it and the
//   post-call commit reads the leaked NORMALIZED value. No suspicious code is
//   involved: review-clean, idiomatic source is silently corrupted. (Triggering
//   shape follows steffahn's upstream reproducer; see ../ledger for details.)
//   Correct: committed credit == declared. Buggy: credit == declared * 1_000_000.
//
// SECURITY IMPACT:
//   The committed credit (the declared 1) is silently replaced by the leaked
//   1_000_000, so the prover produces a VALID proof of a state transition that
//   over-credits the account by ~1e6 units. The verifier accepts a forged
//   balance update / incorrect state root, minting value out of thin air.
// ============================================================================

#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn process_settlement(opening_balance: u32, declared_credit: i32, from: i32, to: i32) -> u32 {
    // Assemble the batch's transaction record from the proven inputs, then run
    // it through the settlement pipeline. `credit` is the committed per-batch
    // credit: equal to `declared_credit` on a correct compiler, but the
    // normalized `declared_credit * 1_000_000` once #137646 leaks the dispatched
    // rule's by-value write back into the caller's record.
    let tx = ledger::Tx { credit: declared_credit, from, to };
    let credit = ledger::settled_amount(tx) as u32;
    opening_balance.wrapping_add(credit)
}
