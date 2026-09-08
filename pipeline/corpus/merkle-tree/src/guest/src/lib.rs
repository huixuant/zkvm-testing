#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
// #[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
#[jolt::provable(max_input_size = 65536, max_output_size = 65536, stack_size = 8388608)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // Byte layout (little-endian is irrelevant; these are raw byte fields):
    //   offset  0..32  : leaf1  (32 bytes)  <- original `leaf1: &[u8]`
    //   offset 32..64  : leaf2  (32 bytes)  <- original `TrustedAdvice<[u8;32]>`
    //   offset 64..96  : leaf3  (32 bytes)  <- original `TrustedAdvice<[u8;32]>`
    //   offset 96..128 : leaf4  (32 bytes)  <- original `UntrustedAdvice<[u8;32]>`
    // Total 128 bytes. The Jolt advice wrappers are host-side plumbing only;
    // the lifted computation reads the raw leaf bytes, matching the original
    // reference which passes 32-byte leaves. DECODE is total: short input
    // returns an empty Vec rather than panicking.
    if input.len() < 128 {
        return Vec::new();
    }
    let leaf1: &[u8] = &input[0..32];
    let leaf2: &[u8] = &input[32..64];
    let leaf3: &[u8] = &input[64..96];
    let leaf4: &[u8] = &input[96..128];

    // (2) CALL -----------------------------------------------------------------
    // Lifted verbatim from merkle-tree's `#[jolt::provable] fn merkle_tree`.
    // Computes the Merkle root of a 4-leaf tree. Byte-for-byte identical to the
    // original body; only the argument source (decoded bytes vs. advice
    // wrappers) changed.
    let h0 = jolt_inlines_sha2::Sha256::digest(leaf1);
    let h1 = jolt_inlines_sha2::Sha256::digest(leaf2);
    let h2 = jolt_inlines_sha2::Sha256::digest(leaf3);
    let h3 = jolt_inlines_sha2::Sha256::digest(leaf4);

    let mut pair_01 = [0u8; 64];
    pair_01[..32].copy_from_slice(&h0);
    pair_01[32..].copy_from_slice(&h1);
    let h01 = jolt_inlines_sha2::Sha256::digest(&pair_01);

    let mut pair_23 = [0u8; 64];
    pair_23[..32].copy_from_slice(&h2);
    pair_23[32..].copy_from_slice(&h3);
    let h23 = jolt_inlines_sha2::Sha256::digest(&pair_23);

    let mut root_pair = [0u8; 64];
    root_pair[..32].copy_from_slice(&h01);
    root_pair[32..].copy_from_slice(&h23);
    let root: [u8; 32] = jolt_inlines_sha2::Sha256::digest(&root_pair);

    // (3) ENCODE ---------------------------------------------------------------
    // The original returns `[u8; 32]`; serialise as its 32 raw bytes.
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&root);
    out
}
