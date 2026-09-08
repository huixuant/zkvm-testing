// ============================================================================
// Test case: issue-127286 / quantized-weight-specialization
// Rust issue: https://github.com/rust-lang/rust/issues/127286
//
// WHAT THIS DOES (real-world zkVM workload):
//   Quantized on-chain ML inference (privacy-preserving ML archetype). The kernel
//   supports two weight-storage formats and uses ONE type-checked `dequant`
//   primitive to dispatch between them:
//     * Structured-weight models store each weight as a packed `(i8, i8)`
//       (mantissa, exponent) pair. Here `dequant::<(i8, i8), (i8, i8)>` is asked to
//       read a `(i8, i8)` as a `(i8, i8)` — the types are EQUAL, so the guard is
//       true and the fast path genuinely runs. This is a real, non-dead use of the
//       primitive; it is what keeps the example from being a no-op factory.
//     * Scalar-quantized models store each weight as a plain `i8`. Here
//       `dequant::<i8, (i8, i8)>` must be DECLINED: an `i8` is not an `(i8, i8)`,
//       so the weight stays scalar and is scored with an ordinary fixed-point
//       multiply. This second call is the miscompilation site.
//   So the same `dequant` is exercised with a matching pair (live) and a
//   mismatching pair (dead) in the same function — the specialization primitive is
//   authentically reused, not conjured solely to host the bug.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   `dequant::<Stored, Wanted>` guards an `unsafe` `core::ptr::read` with the
//   `TypeId::of::<Stored>() == TypeId::of::<Wanted>()` test. The live call
//   `dequant::<(i8, i8), (i8, i8)>` compiles to a correct identity read (guard
//   true) and is unaffected by the bug; monomorphisation is per-instantiation, so
//   it does not change how the *other* instance is folded. The vulnerable instance
//   `dequant::<i8, (i8, i8)>` is the reproducer core with a fresh source
//   type: a `ptr::read` of a 1-byte signed source (`i8`) as a **packed 2-byte**
//   two-element tuple of 1-byte narrow integers `(i8, i8)` — byte-for-byte the
//   same width/layout as the proven `u8 -> (u8, u8)` reproducer (no interior
//   padding; a tight 1-valid + 1-out-of-bounds-byte read), and within the OP's
//   reproducing set (T1 = i8, T2 = i8, T3 = i8). NOTE: a padded/wider target such
//   as `(i8, u16)` (size 4, with padding) did NOT fold on this RISC-V toolchain —
//   the load-bearing detail is the packed 2-byte layout, not merely "a 2-tuple of
//   narrow ints". On the release profile LLVM mishandles the `undef` bytes of the
//   out-of-bounds read on the never-taken branch (regressed in commit #109035) and
//   folds the function to return the read value.
//   Correct: stored `i8` != wanted `(i8, i8)` -> guard false -> `None`.
//   Wrong:   the read is materialised -> `Some((mantissa, exponent))` from garbage.
//
// SECURITY IMPACT:
//   The dtype check is bypassed, so a scalar int8 weight is reinterpreted as a
//   fabricated `(mantissa, exponent)` pair and dequantized to an attacker-influenced
//   magnitude. A valid proof then attests to an inference score computed from
//   forged weights — the verifier accepts a mis-scored classification (e.g. a
//   fraud model flipped below threshold, or a credit score inflated).
// ============================================================================
#![cfg_attr(feature = "guest", no_std)]

use core::any::TypeId;

/// Dequant fast path: reinterpret a stored weight as a richer `Wanted` layout
/// only when the stored dtype provably equals the requested one. Otherwise
/// decline and let the caller use the scalar weight as-is.
fn dequant<Stored: 'static, Wanted: 'static + Copy>(w: Stored) -> Option<Wanted> {
    if TypeId::of::<Stored>() == TypeId::of::<Wanted>() {
        Some(unsafe { core::ptr::read(&w as *const Stored as *const Wanted) })
    } else {
        None
    }
}

#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn score_sample(weight_byte: u8, packed_bytes: [u8; 2], feature: u32) -> u32 {
    // --- Structured-weight model: the stored dtype (i8, i8) EQUALS the requested
    //     (i8, i8), so `dequant`'s guard is true and the fast path genuinely runs.
    //     A correct build always takes the `Some` arm here (identity reinterpret). ---
    let stored_pair: (i8, i8) = (packed_bytes[0] as i8, packed_bytes[1] as i8);
    let structured_term: i32 = match dequant::<(i8, i8), (i8, i8)>(stored_pair) {
        Some((mantissa, exponent)) => (mantissa as i32) << ((exponent as u32) & 7),
        None => 0, // unreachable: (i8, i8) == (i8, i8)
    };

    // --- Scalar-quantized model (the miscompilation site): the stored dtype i8
    //     does NOT equal (i8, i8), so a correct build MUST decline and keep the
    //     weight scalar. ---
    let w: i8 = weight_byte as i8;
    match dequant::<i8, (i8, i8)>(w) {
        Some((mantissa, _exponent)) => {
            // Type confusion from the miscompiled guard: a scalar read as a pair.
            // `mantissa` is byte 0 (== w); combine with the honest structured term.
            // High bit flags that the illegal specialization was (wrongly) taken.
            let forged = (mantissa as i32).wrapping_add(structured_term);
            0x8000_0000 | (forged as u32 & 0x7FFF_FFFF)
        }
        None => {
            // Well-typed: ordinary fixed-point weight * feature, plus the honest
            // structured-weight contribution computed via the live `dequant` call.
            let scalar = (w as i32).wrapping_mul(feature as i32);
            scalar.wrapping_add(structured_term) as u32
        }
    }
}
