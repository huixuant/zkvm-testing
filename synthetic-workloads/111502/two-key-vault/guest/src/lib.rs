// ============================================================================
// Test case: issue-111502 / two-key-vault
// Rust issue: https://github.com/rust-lang/rust/issues/111502
//
// WHAT THIS DOES (real-world zkVM workload):
//   A 2-of-2 multisig vault. Funds are released only if BOTH authorized signers
//   approve the request. The request record carries an `approved` bit; the guest
//   recomputes that bit from the two signatures (`approved = signer_a & signer_b`),
//   stores it back into the record, then reads it back to make the access
//   decision by comparing it against the vault's `required` bit. This is the
//   simplest possible shape of the bug: ONE bit is written, then read back.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   Faithful to the reproducer's aliasing shape. A bundle
//   `(*mut (f32, bool), (f64, bool))` is passed BY VALUE as a function parameter
//   alongside the same raw pointer; a `*mut bool` is derived via
//   `addr_of!(bundle.1.1)`; the recomputed approved bit is written through it as
//   `(a | b) ^ (a ^ b)` (== `a & b`); an otherwise-unused rebinding of the bundle
//   is retained (removing it hides the bug); and finally `bundle.1.1 ==
//   (*policy_ptr).1` is returned. On the 1.94 toolchain `deduce_param_attrs`
//   wrongly marks the by-value `bundle` parameter `readonly`, so LLVM assumes the
//   write through the derived `*mut` cannot change it and folds the read-back to
//   the parameter's INCOMING value (the record's stale `approved` bit) instead of
//   the value just written.
//   Correct: read-back sees the just-written `signer_a & signer_b`.
//   Wrong:   read-back sees the record's stale `approved` bit.
//   These differ whenever `(signer_a & signer_b) != stale_approved` — i.e. for any
//   under-signed request, since the record's stale bit here is `true`.
//
// SECURITY IMPACT:
//   With only one of the two required signatures present, the correct guard denies
//   access, but the miscompiled guard reads the record's stale `approved = true`
//   and RELEASES THE FUNDS. A valid proof authorizes a withdrawal that never had
//   the required 2-of-2 approval — a multisig bypass / theft of vault funds.
// ============================================================================
#![cfg_attr(feature = "guest", no_std)]

use core::ptr::{addr_of, addr_of_mut};

/// A 2-of-2 vault: access is granted only if BOTH authorized signers approve.
fn access_granted(signer_a_ok: bool, signer_b_ok: bool) -> bool {
    // Vault policy behind the pointer: `required = true` (access needs approval).
    let mut policy: (f32, bool) = (0.0, true);
    // Request record: its `approved` bit still holds a stale default (true). It is
    // supposed to be recomputed from the two signatures before it is trusted.
    let request: (f64, bool) = (0.0, true);
    // Bundle: policy pointer paired with the request record.
    let bundle = (addr_of_mut!(policy), request);
    unsafe { recompute_and_decide(bundle.0, bundle, signer_a_ok, signer_b_ok) }
}

unsafe fn recompute_and_decide(
    policy_ptr: *mut (f32, bool),
    bundle: (*mut (f32, bool), (f64, bool)),
    signer_a_ok: bool,
    signer_b_ok: bool,
) -> bool {
    // Recompute the request's `approved` bit in place: BOTH signers must approve.
    // `(a | b) ^ (a ^ b) == a & b`.
    let approved_ptr = addr_of!(bundle.1 .1) as *mut bool;
    let any = signer_a_ok | signer_b_ok;
    let diff = signer_a_ok ^ signer_b_ok;
    (*approved_ptr) = any ^ diff; // == signer_a_ok & signer_b_ok
    let _audit = bundle; // retained for the audit trail (load-bearing)
    // Grant access iff the (recomputed) approved bit matches the required bit.
    return bundle.1 .1 == (*policy_ptr).1;
}

#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn open_vault(amount: u64, signer_a_ok: bool, signer_b_ok: bool) -> u64 {
    if access_granted(signer_a_ok, signer_b_ok) {
        amount | 0x8000_0000_0000_0000 // access granted -> release funds
    } else {
        0 // denied
    }
}
