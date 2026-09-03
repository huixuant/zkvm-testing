// ============================================================================
// Test case: issue-130853 / settlement-idempotency-guard
// Rust issue: https://github.com/rust-lang/rust/issues/130853
//
// WHAT THIS DOES (real-world zkVM workload):
//   A rollup settlement step with a no-op fast path. Applying a batch mutates an
//   account's state cell; a change-detector (`state_changed`) decides whether the
//   state actually advanced. If it did, the guest must commit the freshly
//   recomputed state root; if nothing changed, it takes the idempotent fast path
//   and reuses the cached root. This mirrors how sequencers skip re-hashing
//   unchanged subtrees.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   The state cell is read through a shared `&u8` view, while the opaque
//   `#[inline(never)]` `apply_settlement` writes the SAME byte through a raw
//   `*mut u8` port (`STATE_PORT`) aliasing that view — built with the reproducer's
//   reinterpret dance `&*addr_of!(PTR).cast::<&u8>()`. The 1.94-era GVN MIR pass
//   value-numbers the two dereferences of the shared reference as equal and
//   assumes the opaque call cannot mutate the pointee (the aliasing hole fixed by
//   PR #133474), so it replaces the post-call re-read with the pre-call value.
//   Correct: `before = 0`, settlement writes a non-zero byte, re-read differs, so
//   `**cell != before` = true  -> state advanced.
//   Wrong:   the re-read is folded to `before`, so `before != before` = false
//   -> state reads unchanged.
//
// SECURITY IMPACT:
//   A settlement that really moved funds is misclassified as a no-op, so the
//   guest commits the *pre-settlement* cached root while the debit was applied.
//   A valid proof attests to a state root that omits an applied balance change —
//   the spender's debit vanishes from the committed state (off-book credit).
// ============================================================================
#![cfg_attr(feature = "guest", no_std)]

use core::ptr::{addr_of, addr_of_mut};

/// Raw port aliasing the account's low state byte, written by the settlement step.
static mut STATE_PORT: *mut u8 = core::ptr::null_mut();

/// Opaque settlement step. `#[inline(never)]` hides the store; it writes the
/// account's new low state byte through the port.
#[inline(never)]
unsafe fn apply_settlement(new_low: u8) {
    *STATE_PORT = new_low;
}

/// Did the account state actually change after settlement? `cell` is a shared
/// `&u8` view of the account's low state byte.
unsafe fn state_changed(cell: &&u8, new_low: u8) -> bool {
    let before = **cell;
    apply_settlement(new_low); // opaque: mutates the cell through STATE_PORT
    **cell != before
}

#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn settle_batch(new_state_byte: u8, applied_root: u32, cached_root: u32) -> u32 {
    // Account low state byte on the stack; STATE_PORT aliases it.
    let mut state_low: u8 = 0;

    unsafe {
        STATE_PORT = addr_of_mut!(state_low);
        let cell: &&u8 = &*addr_of!(STATE_PORT).cast::<&u8>();

        if state_changed(cell, new_state_byte) {
            // State advanced: commit the recomputed post-settlement root.
            applied_root
        } else {
            // Idempotent fast path: nothing changed, reuse the cached root.
            cached_root
        }
    }
}
