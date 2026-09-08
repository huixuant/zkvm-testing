#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

use jolt::{end_cycle_tracking, start_cycle_tracking};

use secp256k1::{
    ecdsa::{RecoverableSignature, RecoveryId},
    Message, PublicKey,
};

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test.
///
/// Mode B (benchmark lift) of `corpus/src/recover-ecdsa`. The original provable
/// function was
///
///     #[jolt::provable(...)]
///     fn recover(sig: &[u8], msg: [u8; 32]) -> PublicKey
///
/// whose body is an ECDSA public-key recovery over secp256k1. Its two arguments
/// are reconstructed from `input` and its `PublicKey` return value is
/// serialised. The computation between DECODE and ENCODE is byte-for-byte the
/// original body (including its cycle-tracking markers, which are host-side
/// side channels only and never influence the returned bytes).
///
/// DECODE byte layout (total, panic-free). Minimum length 97 bytes:
///   input[0..64]   -> sig[0..64] : compact recoverable ECDSA signature, r || s,
///                                  each a 32-byte BIG-ENDIAN scalar
///   input[64]      -> sig[64]    : recovery id, u8, valid values 0..=3
///   input[65..97]  -> msg        : [u8; 32] message digest (opaque 32 bytes)
///   input[97..]                  : ignored (extra trailing bytes are allowed)
///
/// Totality rules (all deterministic, identical on both compiler arms):
///   * `input.len() < 97`                      -> return an empty Vec.
///   * recovery id outside 0..=3               -> return an empty Vec.
///   * `RecoverableSignature::from_compact` Err -> return an empty Vec.
///   * `recover_ecdsa` Err (no such key)       -> return an empty Vec.
/// These replace the original's `.unwrap()`s so a malformed input costs one
/// empty output instead of a guest panic; the success path is unchanged.
///
/// ENCODE: the recovered `PublicKey` in SEC1 UNCOMPRESSED form, i.e. exactly 65
/// bytes `0x04 || X (32B BE) || Y (32B BE)`, via `PublicKey::serialize_uncompressed()`.
/// So output is either 0 bytes (failure) or 65 bytes (success).
#[jolt::provable(stack_size = 8388608, heap_size = 16777216, max_trace_length = 1048576)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    if input.len() < 97 {
        return Vec::new();
    }
    let sig: &[u8] = &input[0..65];
    let mut msg = [0u8; 32];
    msg.copy_from_slice(&input[65..97]);

    // (2) CALL -----------------------------------------------------------------
    // Original `recover` provable-fn body, lifted; only the `.unwrap()`s become
    // early returns so DECODE stays total.
    use secp256k1::Secp256k1;

    start_cycle_tracking("recover");
    let recovery_id = match RecoveryId::try_from(sig[64] as i32) {
        Ok(id) => id,
        Err(_) => return Vec::new(),
    };
    let sig = match RecoverableSignature::from_compact(&sig[0..64], recovery_id) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let secp = Secp256k1::new();
    let public: PublicKey = match secp.recover_ecdsa(Message::from_digest(msg), &sig) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    end_cycle_tracking("recover");

    // (3) ENCODE ---------------------------------------------------------------
    let mut out = Vec::with_capacity(65);
    out.extend_from_slice(&public.serialize_uncompressed());
    out
}
