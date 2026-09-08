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
    // Lifted from merkle-tree-guest's `merkle_tree(leaf1, leaf2, leaf3, leaf4)`.
    // The original four arguments (leaf1: &[u8], and leaf2/leaf3/leaf4 wrapped in
    // Trusted/UntrustedAdvice over [u8; 32]) are reconstructed from `input`.
    //
    // Byte layout (little-endian order of concatenation, 128 bytes total):
    //   input[0..32]    -> leaf1  ([u8; 32])
    //   input[32..64]   -> leaf2  ([u8; 32])
    //   input[64..96]   -> leaf3  ([u8; 32])
    //   input[96..128]  -> leaf4  ([u8; 32])
    // DECODE is total: if fewer than 128 bytes are supplied, return an empty Vec
    // rather than panicking.
    if input.len() < 128 {
        return Vec::new();
    }
    let leaf1: &[u8] = &input[0..32];
    let leaf2: &[u8] = &input[32..64];
    let leaf3: &[u8] = &input[64..96];
    let leaf4: &[u8] = &input[96..128];

    // (2) CALL -----------------------------------------------------------------
    // Body lifted byte-for-byte from merkle_tree(): compute the 4-leaf Merkle
    // root using the SHA-256 inline. The Advice wrappers only provided Deref to
    // the underlying [u8; 32]; here the leaves are already plain byte slices.
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
    let root = jolt_inlines_sha2::Sha256::digest(&root_pair);

    // (3) ENCODE ---------------------------------------------------------------
    // Serialize the 32-byte Merkle root.
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&root);
    out
}
