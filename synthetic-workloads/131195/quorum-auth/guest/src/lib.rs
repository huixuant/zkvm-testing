// ============================================================================
// Test case: issue-131195 / quorum-authorization
// Rust issue: https://github.com/rust-lang/rust/issues/131195
//
// WHAT THIS DOES (real-world zkVM workload):
//   A verifiable multisig / governance authorization check of the kind a treasury
//   or DAO executor runs inside the circuit before releasing an action. A board of
//   32 signer seats is tracked by a `u32` approval bitmask (bit i set = seat i has
//   authorised the action). Each *proposal class* carries a fixed standing
//   pre-delegation: some seats are permanently pre-authorised for that class of
//   action (e.g. automated custody seats, an emergency council). An action may
//   execute immediately only when the standing delegation ALREADY covers the whole
//   board — every seat is pre-authorised, so no fresh live signature is needed.
//   Otherwise the executor holds and waits for live signatures to be collected off
//   this fast path.
//
//   This is a *grant-direction*, genuine-bitmask sibling of `bitset-slot-allocator-
//   bitmask`. There the trigger idiom was "the reservation does not saturate all
//   slots" (`!mask != 0`, a liveness/refuse flip). Here the idiom is the dual
//   saturation test "the standing delegation covers every seat" (`!mask == 0`, an
//   authorise/execute flip), which is the natural home for a fixed-quorum check.
//
//   Note on the design: BOTH branches are individually safe — auto-execute only
//   when the class is fully pre-delegated, otherwise hold for live signatures.
//   Nothing in the source logic authorises an under-quorum action; the danger
//   below comes solely from the miscompilation routing a partially-delegated class
//   into the auto-execute branch.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   The load-bearing trigger for issue #131195 is a bitwise `!c` fed into an
//   equality against 0, on an integer whose value JumpThreading can *pin to a
//   constant along an edge*. The pin here is natural control flow: a `match` on the
//   runtime `proposal_class` maps each class onto a fixed pre-delegation mask, so on
//   every match arm `approvals` is a concrete constant flowing into `!approvals`.
//   The 1.94-era JumpThreading pass threads that value but treats the bitwise NOT
//   as boolean (the unsoundness fixed by PR #131203 / #131201), folding the
//   saturation test `!approvals == 0` into `approvals != 0`.
//   Correct (class 1, `approvals = 0x0000_00FF`): `!0x0000_00FF == 0`
//   = `0xFFFF_FF00 == 0` = false -> board not fully delegated -> hold.
//   Wrong: the pass folds it to `0x0000_00FF != 0` = true -> the partial standing
//   delegation is misread as full board coverage -> the action auto-executes.
//   (Classes whose mask is `0` or all-ones agree under both compilers; the
//   divergence comes only from partial masks — the treasury class here.)
//
// SECURITY IMPACT:
//   A treasury action that has only a minority of seats pre-delegated (and no live
//   signatures yet) is misreported as fully authorised, so the circuit emits a
//   valid proof executing it under quorum. The verifier accepts a forged
//   authorization: funds move / privileged action fires without the required
//   signatures ever being collected — a governance / multisig bypass.
// ============================================================================
#![cfg_attr(feature = "guest", no_std)]

/// Authorization token: high bit = "executed"; bits 24..30 carry the count of
/// pre-delegated seats (for audit); low 24 bits = the action id. `0` means "held".
fn execute(approvals: u32, action_id: u32) -> u32 {
    let seats = approvals.count_ones() & 0x3F;
    0x8000_0000 | (seats << 24) | (action_id & 0x00FF_FFFF)
}

#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn authorize_action(proposal_class: u32, action_id: u32) -> u32 {
    // Each proposal class carries a fixed standing pre-delegation over the 32
    // seats. The `match` pins `approvals` to a concrete mask on each arm.
    let approvals: u32 = match proposal_class {
        0 => 0xFFFF_FFFF, // fast-track: every seat standing-delegated
        1 => 0x0000_00FF, // treasury: only the 8 automated custody seats
        2 => 0x0000_0000, // standard: nothing pre-delegated
        _ => 0x0000_0000, // unknown class: treat as no standing delegation
    };

    // May this action execute on the standing-delegation fast path? Only when the
    // pre-delegation already covers *every* seat, i.e. the approval mask is
    // saturated -- equivalently its bitwise complement is zero.
    let missing = !approvals; // seats still lacking a standing delegation
    if missing == 0 {
        // Board fully pre-delegated for this class: authorise immediately.
        execute(approvals, action_id)
    } else {
        // Not fully delegated: hold and wait for live signatures off this path.
        0
    }
}
