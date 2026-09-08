#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

use jolt_inlines_p256::{ecdsa_verify, P256Error, P256Fr, P256Point};

/// Map a `P256Error` to a stable 1-byte code so the outcome is observable in the
/// harness output instead of only via the panic flag. Pure plumbing: no algorithm
/// change, no toolchain-conditional code.
#[inline(always)]
fn err_code(e: &P256Error) -> u8 {
    match e {
        P256Error::InvalidFqElement => 1,
        P256Error::InvalidFrElement => 2,
        P256Error::NotOnCurve => 3,
        P256Error::QAtInfinity => 4,
        P256Error::ROrSZero => 5,
        P256Error::RxMismatch => 6,
        P256Error::InvalidGlvSignWord(_) => 7,
    }
}

/// The lifted body of the original benchmark's provable function
/// (`p256_ecdsa_verify` in corpus/src/p256-ecdsa-verify/guest/src/lib.rs).
///
/// The original used `.unwrap_or_spoil_proof()` on each `Result`, i.e. it turned
/// every failure into a panic and returned `()`. Only that *reporting* step is
/// re-plumbed here: the exact same constructors and the exact same
/// `ecdsa_verify` call are made, in the same order, on the same values; the
/// `Result`s are reported as bytes rather than panicked on. The computation
/// between decode and encode is byte-for-byte the original.
///
/// `#[inline(never)]` so the function survives `lto = "fat"` + `-Copt-level=3`
/// as its own symbol and `--trace-fn` can resolve it.
#[inline(never)]
fn run_p256_ecdsa_verify(z: [u64; 4], r: [u64; 4], s: [u64; 4], q: [u64; 8]) -> (u8, u8) {
    // Stage 0x02..0x05: input well-formedness (originally `unwrap_or_spoil_proof`).
    let z = match P256Fr::from_u64_arr(&z) {
        Ok(v) => v,
        Err(e) => return (0x02, err_code(&e)),
    };
    let r = match P256Fr::from_u64_arr(&r) {
        Ok(v) => v,
        Err(e) => return (0x03, err_code(&e)),
    };
    let s = match P256Fr::from_u64_arr(&s) {
        Ok(v) => v,
        Err(e) => return (0x04, err_code(&e)),
    };
    // The following call ensures that q is a valid point on the P256 curve
    let q = match P256Point::from_u64_arr(&q) {
        Ok(v) => v,
        Err(e) => return (0x05, err_code(&e)),
    };
    // Perform the ECDSA verification knowing all inputs are well-formed
    match ecdsa_verify(z, r, s, q) {
        Ok(()) => (0x00, 0x00),
        Err(e) => (0x01, err_code(&e)),
    }
}

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(heap_size = 32768, max_trace_length = 65536, stack_size = 8388608)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    //
    // BYTE LAYOUT (little-endian throughout), exactly 160 bytes required:
    //
    //   offset  size  field
    //   ------  ----  ----------------------------------------------------------
    //     0      32   z : [u64; 4]  message hash, scalar-field element (mod n)
    //    32      32   r : [u64; 4]  signature r, scalar-field element (mod n)
    //    64      32   s : [u64; 4]  signature s, scalar-field element (mod n)
    //    96      64   q : [u64; 8]  public key, Qx[u64;4] || Qy[u64;4] (mod p)
    //
    // Each [u64; N] is N consecutive little-endian u64s: limb i occupies bytes
    // [8*i, 8*i+8) of that field, so the whole 160-byte block is simply the
    // limb arrays concatenated in LE order.
    //
    // Bytes beyond offset 160 are ignored. If `input.len() < 160` the harness
    // returns an EMPTY Vec (total decode, never panics).
    //
    // Known-good vector (RFC 6979 P-256, SHA-256("sample")) as 160 hex bytes,
    // useful as a generator seed — mutating any limb of z/r/s still reaches
    // `ecdsa_verify`'s full Fake-GLV path and reports RxMismatch:
    //   z = df8e7c30407c9f21 56f6d87a850af383 c16784d74b36d606 8ae61fc24abe4748
    //   r = 7ce80a972e8aba61 b85ab0e6f84617f8 c8c64f0f9a9eab15 7dbe86dea75bed42
    //   s = d6b41feb71a214de c7dfd7b5f17980bb cd7a97db7e0d8886 bf8f314588aaf881
    //   q = b69ff2602e6269e6 6cfa613b92b849c0 686d35c674eb61c9 319d5a25bad4fe60
    //       992246d494c2a377 519f7e2d0cb2f1f2 64bc2856e9e91aa4 99bcb80810fe0379
    // (each 16-hex group above is one u64 written in the byte order it appears
    //  in the file, i.e. already little-endian.)
    const NEEDED: usize = 160;
    if input.len() < NEEDED {
        return Vec::new();
    }

    #[inline(always)]
    fn limb(buf: &[u8], off: usize) -> u64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&buf[off..off + 8]);
        u64::from_le_bytes(b)
    }

    let mut z = [0u64; 4];
    let mut r = [0u64; 4];
    let mut s = [0u64; 4];
    let mut q = [0u64; 8];
    let mut i = 0;
    while i < 4 {
        z[i] = limb(&input, i * 8);
        r[i] = limb(&input, 32 + i * 8);
        s[i] = limb(&input, 64 + i * 8);
        i += 1;
    }
    let mut j = 0;
    while j < 8 {
        q[j] = limb(&input, 96 + j * 8);
        j += 1;
    }

    // (2) CALL -----------------------------------------------------------------
    let (stage, code) = run_p256_ecdsa_verify(z, r, s, q);

    // (3) ENCODE ---------------------------------------------------------------
    //
    // OUTPUT LAYOUT: exactly 2 bytes (or 0 bytes on short input).
    //   byte 0: stage — 0x00 signature VALID
    //                   0x01 ecdsa_verify returned Err
    //                   0x02 z not a canonical Fr element
    //                   0x03 r not a canonical Fr element
    //                   0x04 s not a canonical Fr element
    //                   0x05 q not a valid curve point / not canonical Fq
    //   byte 1: P256Error discriminant (0 when stage == 0x00):
    //           1 InvalidFqElement, 2 InvalidFrElement, 3 NotOnCurve,
    //           4 QAtInfinity, 5 ROrSZero, 6 RxMismatch, 7 InvalidGlvSignWord
    let mut out = Vec::with_capacity(2);
    out.push(stage);
    out.push(code);
    out
}
